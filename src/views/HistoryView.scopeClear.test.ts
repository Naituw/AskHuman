import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryEntry, ProjectInfo } from "../lib/types";

const mocks = vi.hoisted(() => ({
  entries: [] as unknown[],
  projects: [] as unknown[],
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => undefined),
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(name, handler);
    return () => {
      mocks.listeners.delete(name);
    };
  }),
}));
vi.mock("@tauri-apps/api/webviewWindow", () => ({ getCurrentWebviewWindow: () => ({ close: vi.fn(async () => {}) }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn(async () => {}) }));
vi.mock("../lib/ipc", () => ({
  clearAllHistory: vi.fn(async () => 0),
  deleteHistoryEntries: vi.fn(async () => 0),
  getHistory: vi.fn(async () => mocks.entries),
  getHistoryProjects: vi.fn(async () => mocks.projects),
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

function entryOf(id: string, name: string, session: number, text: string, files: string[] = []): HistoryEntry {
  return {
    id,
    timestampMs: 0,
    project: `/p/${name}`,
    source: "",
    agentKind: "cursor",
    agentSessionId: `${name}-session-${session}`,
    mcpInstanceId: null,
    channel: "popup",
    action: "send",
    isMarkdown: false,
    message: {
      text,
      files: files.map((path) => ({ path, name: path.split("/").pop()!, size: 5, isImage: false })),
    },
    questions: [],
    answers: [],
  };
}

// Newest first, as the backend returns them. The newest entry of all belongs to another session than alpha's first.
const ENTRIES = () =>
  [
    entryOf("bravo-0", "bravo", 0, "bravo zeta"),
    entryOf("alpha-0a", "alpha", 0, "alpha zeta", ["/q/a.md"]),
    entryOf("alpha-0b", "alpha", 0, "alpha plain"),
    entryOf("charlie-0", "charlie", 0, "charlie zeta", ["/q/b.md"]),
    entryOf("alpha-1", "alpha", 1, "alpha other"),
  ].map((entry, index) => ({ ...entry, timestampMs: 1_700_000_000_000 - index * 1000 }));

const EVERYTHING = ["bravo zeta", "alpha zeta", "alpha plain", "charlie zeta", "alpha other"];
const EVERYTHING_LABEL = "All projects · All sessions";

let wrapper: VueWrapper | undefined;
async function mountHistory(url = "/?all=1") {
  window.history.replaceState({}, "", url);
  wrapper = mount(HistoryView, { attachTo: document.body, global: { plugins: [i18n] } });
  await flushPromises();
  await flushPromises();
  return wrapper;
}
async function settle() {
  await flushPromises();
  await nextTick();
}

const summaries = (view: VueWrapper) => view.findAll(".entry").map((item) => item.find(".entry-summary").text());
const activeSummary = (view: VueWrapper) => view.get(".entry.active .entry-summary").text();
const scopeLabel = (view: VueWrapper) => view.get(".scope-btn-label").text();
const clearButton = (view: VueWrapper) => view.find(".scope-clear");

/** What the popup's "history" button does to a window that is open: re-target it on the asking session. */
async function openFromPopup(sessionId = "alpha-session-0") {
  mocks.listeners.get("history-open-target")!({
    payload: { all: false, project: "/p/alpha", target: { type: "agent", agentKind: "cursor", sessionId } },
  });
  await settle();
  await settle();
}
async function typeQuery(view: VueWrapper, text: string) {
  await view.get(".search-input").setValue(text);
  await settle();
}
async function pickSuffix(view: VueWrapper, label: string) {
  await view.get(".attach-btn").trigger("click");
  await settle();
  const row = view.findAll(".attach-ext").find((item) => item.find(".attach-ext-name").text() === label);
  if (!row) throw new Error(`no suffix row ${label}`);
  await row.get("input").trigger("click");
  await settle();
}

describe("history scope: clearing it in one step", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listeners.clear();
    document.body.replaceChildren();
    i18n.global.locale.value = "en";
    mocks.projects = ["alpha", "bravo", "charlie"].map<ProjectInfo>((name, i) => ({
      key: `/p/${name}`,
      name,
      count: name === "alpha" ? 3 : 1,
      lastMs: 1_700_000_000_000 - i,
    }));
    mocks.entries = ENTRIES();
  });
  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
    window.history.replaceState({}, "", "/");
  });

  it("has nothing to clear while the scope is everything", async () => {
    const view = await mountHistory();
    expect(scopeLabel(view)).toBe(EVERYTHING_LABEL);
    expect(clearButton(view).exists()).toBe(false);
    expect(view.get(".scope-btn").classes()).not.toContain("clearable");
    expect(summaries(view)).toEqual(EVERYTHING);
  });

  it("goes from the asking session back to everything, and selects the newest entry", async () => {
    const view = await mountHistory();
    await openFromPopup();
    expect(scopeLabel(view)).toContain("alpha-se");
    expect(summaries(view)).toEqual(["alpha zeta", "alpha plain"]);
    expect(activeSummary(view)).toBe("alpha zeta");

    const clear = clearButton(view);
    expect(clear.exists()).toBe(true);
    // Its own button next to the scope button, not inside it, and named for assistive technology.
    expect(view.find(".scope-btn .scope-clear").exists()).toBe(false);
    expect(clear.attributes("aria-label")).toBe("Clear scope, show everything");
    expect(clear.attributes("title")).toBe("Clear scope, show everything");
    expect(view.get(".scope-btn").classes()).toContain("clearable");

    await clear.trigger("click");
    await settle();
    expect(scopeLabel(view)).toBe(EVERYTHING_LABEL);
    expect(summaries(view)).toEqual(EVERYTHING);
    expect(activeSummary(view)).toBe("bravo zeta");
    expect(clearButton(view).exists()).toBe(false);
    expect(view.get(".scope-btn").classes()).not.toContain("clearable");
    // It clears; it does not open the menu.
    expect(view.find(".scope-menu").exists()).toBe(false);
  });

  it("clears a project scope too, as when the window opens on the current project", async () => {
    const view = await mountHistory("/?project=/p/alpha&projectName=alpha");
    expect(scopeLabel(view)).toBe("alpha · All sessions");
    expect(summaries(view)).toEqual(["alpha zeta", "alpha plain", "alpha other"]);

    await view.get(".scope-clear").trigger("click");
    await settle();
    expect(scopeLabel(view)).toBe(EVERYTHING_LABEL);
    expect(summaries(view)).toEqual(EVERYTHING);
    expect(clearButton(view).exists()).toBe(false);
  });

  it("clears a scope that was picked in the menu", async () => {
    const view = await mountHistory();
    await view.get(".scope-btn").trigger("click");
    await settle();
    // The first row is "All sessions"; the next ones are single sessions.
    await view.findAll(".scope-session-menu button")[1].trigger("click");
    await settle();
    expect(view.find(".scope-menu").exists()).toBe(false);
    expect(clearButton(view).exists()).toBe(true);
    expect(scopeLabel(view)).not.toBe(EVERYTHING_LABEL);

    await view.get(".scope-clear").trigger("click");
    await settle();
    expect(scopeLabel(view)).toBe(EVERYTHING_LABEL);
    expect(summaries(view)).toEqual(EVERYTHING);
    expect(clearButton(view).exists()).toBe(false);
  });

  it("keeps the keywords and the attachment conditions, which have clear controls of their own", async () => {
    const view = await mountHistory();
    await openFromPopup();
    await typeQuery(view, "zeta");
    await pickSuffix(view, ".md");
    expect(summaries(view)).toEqual(["alpha zeta"]);
    expect(view.findAll(".attach-chip")).toHaveLength(1);

    await view.get(".scope-clear").trigger("click");
    await settle();
    // Everything with the keyword and a Markdown attachment: alpha's entry and charlie's, not bravo's (no attachment).
    expect(summaries(view)).toEqual(["alpha zeta", "charlie zeta"]);
    expect((view.get(".search-input").element as HTMLInputElement).value).toBe("zeta");
    expect(view.findAll(".attach-chip")).toHaveLength(1);
  });

  it("closes the scope menu if it is open", async () => {
    const view = await mountHistory();
    await openFromPopup();
    await view.get(".scope-btn").trigger("click");
    await settle();
    expect(view.find(".scope-menu").exists()).toBe(true);

    await view.get(".scope-clear").trigger("click");
    await settle();
    expect(view.find(".scope-menu").exists()).toBe(false);
    expect(scopeLabel(view)).toBe(EVERYTHING_LABEL);
  });

  it("hands the keyboard focus to the scope button when the clear button had it, and leaves it alone otherwise", async () => {
    const view = await mountHistory();
    await openFromPopup();
    (view.get(".scope-clear").element as HTMLButtonElement).focus();
    expect(document.activeElement).toBe(view.get(".scope-clear").element);
    await view.get(".scope-clear").trigger("click");
    await settle();
    expect(document.activeElement).toBe(view.get(".scope-btn").element);

    // Pressed without ever taking focus (a click in a webview that does not focus buttons): nothing is moved.
    (document.activeElement as HTMLElement).blur();
    await openFromPopup();
    await view.get(".scope-clear").trigger("click");
    await settle();
    expect(document.activeElement).not.toBe(view.get(".scope-btn").element);
  });

  it("is named in the interface language", async () => {
    vi.mocked(historyInit).mockResolvedValueOnce({ theme: "light", lang: "zh", project: "", projectName: "" });
    const view = await mountHistory();
    await openFromPopup();
    expect(view.get(".scope-clear").attributes("aria-label")).toBe("清除范围，显示全部");
    expect(view.get(".scope-clear").attributes("title")).toBe("清除范围，显示全部");
  });
});
