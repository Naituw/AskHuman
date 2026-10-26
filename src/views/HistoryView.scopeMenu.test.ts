import { flushPromises, mount, type DOMWrapper, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HistoryEntry, ProjectInfo } from "../lib/types";

const mocks = vi.hoisted(() => ({
  entries: [] as unknown[],
  projects: [] as unknown[],
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => undefined),
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
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
import { i18n } from "../i18n";
import { STALL_MS } from "./history/scopeMenuIntent";

const NAMES = ["alpha", "bravo", "charlie", "delta", "echo"];

function entryOf(name: string, session: number, n: number): HistoryEntry {
  return {
    id: `${name}-${session}-${n}`,
    timestampMs: 1_700_000_000_000 - n,
    project: `/p/${name}`,
    source: "",
    agentKind: "cursor",
    agentSessionId: `${name}-session-${session}`,
    mcpInstanceId: null,
    channel: "popup",
    action: "send",
    isMarkdown: false,
    message: { text: `${name} ${session} ${n}`, files: [] },
    questions: [],
    answers: [],
  };
}

// The menu geometry the guard sees: project rows are 32px tall in a column left of x = 384 (row i is
// centred on y = 85 + 32 i, row 0 being "All projects"); the session column starts at x = 384.
const rowY = (i: number) => 85 + i * 32;
const PANE = { left: 384, top: 65, right: 734, bottom: 361, width: 350, height: 296, x: 384, y: 65 } as DOMRect;
const NO_RECT = { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0, x: 0, y: 0 } as DOMRect;

let wrapper: VueWrapper | undefined;

async function mountHistory() {
  wrapper = mount(HistoryView, { attachTo: document.body, global: { plugins: [i18n] } });
  await flushPromises();
  await flushPromises();
  return wrapper;
}
async function settle() {
  await flushPromises();
  await nextTick();
}
async function openMenu(view: VueWrapper) {
  await view.get(".scope-btn").trigger("click");
  await settle();
}

const rows = (view: VueWrapper) => view.findAll(".scope-project-menu button");
const labelOf = (row: DOMWrapper<Element>) => row.get(".scope-project-label").text().replace(/ \(\d+\)$/, "");
const rowOf = (view: VueWrapper, name: string) => {
  const row = rows(view).find((item) => labelOf(item) === name);
  if (!row) throw new Error(`no project row ${name}; have ${rows(view).map(labelOf).join(", ")}`);
  return row;
};
const shownProject = (view: VueWrapper) => {
  const row = view.find(".scope-project-menu button.active");
  return row.exists() ? labelOf(row) : "";
};
const titleOf = (view: VueWrapper) => view.get(".scope-session-title-text").text();
const guarding = (view: VueWrapper) => view.get(".scope-menu").classes().includes("guarding");

/** Dispatch a pointer event (VTU's `trigger` cannot carry coordinates for pointer events). */
async function fire(target: Element, type: string, x: number, y: number, pointerType = "mouse") {
  const Ctor = (window as unknown as { PointerEvent?: typeof MouseEvent }).PointerEvent ?? MouseEvent;
  const event = new Ctor(type, {
    bubbles: type === "pointermove", // enter/leave do not bubble, exactly like the browser's
    cancelable: true,
    clientX: x,
    clientY: y,
    ...({ pointerType } as object),
  });
  if ((event as { pointerType?: string }).pointerType === undefined) {
    Object.defineProperty(event, "pointerType", { value: pointerType });
  }
  target.dispatchEvent(event);
  await nextTick();
}

/** A mouse over the project rows: leave/enter notifications like the browser sends, then a move. */
function mouseOver(view: VueWrapper) {
  let on: DOMWrapper<Element> | null = null;
  return {
    async to(name: string, x: number, y: number, pointerType = "mouse") {
      const row = rowOf(view, name);
      if (on && on.element !== row.element) await fire(on.element, "pointerleave", x, y, pointerType);
      if (!on || on.element !== row.element) await fire(row.element, "pointerenter", x, y, pointerType);
      on = row;
      await fire(row.element, "pointermove", x, y, pointerType);
    },
    async intoSessionColumn() {
      if (on) await fire(on.element, "pointerleave", PANE.left, rowY(3));
      on = null;
      await fire(view.get(".scope-session-menu").element, "pointerenter", PANE.left + 1, rowY(3));
    },
  };
}

describe("history scope menu pointer intent", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    document.body.replaceChildren();
    window.history.replaceState({}, "", "/?all=1");
    i18n.global.locale.value = "en";
    mocks.projects = NAMES.map<ProjectInfo>((name, i) => ({
      key: `/p/${name}`,
      name,
      count: 4,
      lastMs: 1_700_000_000_000 - i,
    }));
    mocks.entries = NAMES.flatMap((name) => [0, 1].flatMap((session) => [0, 1].map((n) => entryOf(name, session, n))));
    vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
      return this.classList.contains("scope-session-menu") ? PANE : NO_RECT;
    });
  });
  afterEach(() => {
    wrapper?.unmount();
    wrapper = undefined;
    vi.useRealTimers();
    vi.restoreAllMocks();
    window.history.replaceState({}, "", "/");
  });

  it("names the project the session column belongs to", async () => {
    const view = await mountHistory();
    await openMenu(view);
    expect(titleOf(view)).toBe("All projects");

    await mouseOver(view).to("bravo", 260, rowY(2));
    expect(shownProject(view)).toBe("bravo");
    expect(titleOf(view)).toBe("bravo");
    expect(view.get(".scope-session-title").attributes("title")).toBe("/p/bravo");
  });

  it("keeps the hovered project while the pointer crosses other rows on the way to a session", async () => {
    const view = await mountHistory();
    await openMenu(view);
    const mouse = mouseOver(view);
    await mouse.to("echo", 260, rowY(5)); // first contact shows echo at once
    expect(shownProject(view)).toBe("echo");
    await mouse.to("echo", 270, rowY(5) - 8); // still on echo: the pointer sets off from here

    await mouse.to("delta", 300, rowY(4)); // up and to the right, across delta and charlie
    await mouse.to("charlie", 340, rowY(3) + 6);
    expect(shownProject(view)).toBe("echo");
    expect(titleOf(view)).toBe("echo");
    expect(guarding(view)).toBe(true);

    await mouse.intoSessionColumn();
    expect(guarding(view)).toBe(false);
    expect(shownProject(view)).toBe("echo");

    // Clicking a session commits the project it is listed under, not the last row the pointer crossed.
    await view.get(".scope-session-menu button").trigger("click");
    await settle();
    expect(view.find(".scope-menu").exists()).toBe(false);
    expect(view.get(".scope-btn-label").text()).toBe("echo · All sessions");
  });

  it("switches at once when the pointer moves straight along the project column", async () => {
    const view = await mountHistory();
    await openMenu(view);
    const mouse = mouseOver(view);
    await mouse.to("echo", 260, rowY(5));
    await mouse.to("echo", 260, rowY(5) - 8);
    await mouse.to("delta", 260, rowY(4) + 4);
    expect(shownProject(view)).toBe("delta");
    expect(guarding(view)).toBe(false);
  });

  it("switches to a crossed row once the pointer rests on it", async () => {
    const view = await mountHistory();
    await openMenu(view);
    const mouse = mouseOver(view);
    await mouse.to("echo", 260, rowY(5));
    await mouse.to("echo", 270, rowY(5) - 8);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    await mouse.to("delta", 300, rowY(4));
    expect(shownProject(view)).toBe("echo");
    expect(guarding(view)).toBe(true);

    vi.advanceTimersByTime(STALL_MS);
    await nextTick();
    expect(shownProject(view)).toBe("delta");
    expect(titleOf(view)).toBe("delta");
    expect(guarding(view)).toBe(false);
  });

  it("does not switch to a crossed row the pointer has already left", async () => {
    const view = await mountHistory();
    await openMenu(view);
    const mouse = mouseOver(view);
    await mouse.to("echo", 260, rowY(5));
    await mouse.to("echo", 270, rowY(5) - 8);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    await mouse.to("delta", 300, rowY(4));
    expect(guarding(view)).toBe(true);

    // Off delta and onto no other row (e.g. the column padding): nothing is left to switch to.
    await fire(rowOf(view, "delta").element, "pointerleave", 300, rowY(4) - 20);
    vi.advanceTimersByTime(5000);
    await nextTick();
    expect(shownProject(view)).toBe("echo");
    expect(guarding(view)).toBe(false);
  });

  it("does not highlight a crossed row as chosen while a switch is deferred", async () => {
    const view = await mountHistory();
    await openMenu(view);
    const mouse = mouseOver(view);
    await mouse.to("echo", 260, rowY(5));
    await mouse.to("echo", 270, rowY(5) - 8);
    await mouse.to("delta", 300, rowY(4));
    const active = view.findAll(".scope-project-menu button.active");
    expect(active).toHaveLength(1);
    expect(labelOf(active[0])).toBe("echo");
  });

  it("treats a click on a crossed row as an explicit choice", async () => {
    const view = await mountHistory();
    await openMenu(view);
    const mouse = mouseOver(view);
    await mouse.to("echo", 260, rowY(5));
    await mouse.to("echo", 270, rowY(5) - 8);
    await mouse.to("delta", 300, rowY(4));
    expect(guarding(view)).toBe(true);
    await rowOf(view, "delta").trigger("click");
    expect(shownProject(view)).toBe("delta");
    expect(guarding(view)).toBe(false);
  });

  it("shows a project at once when it takes keyboard focus", async () => {
    const view = await mountHistory();
    await openMenu(view);
    await rowOf(view, "charlie").trigger("focus");
    expect(shownProject(view)).toBe("charlie");
    expect(titleOf(view)).toBe("charlie");
  });

  it("treats a touch pointer as an explicit choice", async () => {
    const view = await mountHistory();
    await openMenu(view);
    const mouse = mouseOver(view);
    await mouse.to("echo", 260, rowY(5), "touch");
    await mouse.to("delta", 300, rowY(4), "touch");
    expect(shownProject(view)).toBe("delta");
    expect(guarding(view)).toBe(false);
  });

  it("starts clean when the menu is closed and opened again", async () => {
    const view = await mountHistory();
    await openMenu(view);
    const mouse = mouseOver(view);
    await mouse.to("echo", 260, rowY(5));
    await mouse.to("echo", 270, rowY(5) - 8);
    await mouse.to("delta", 300, rowY(4));
    expect(guarding(view)).toBe(true);

    await view.get(".scope-btn").trigger("click"); // close
    await settle();
    await openMenu(view); // reopen: back to the applied scope, nothing deferred
    expect(guarding(view)).toBe(false);
    expect(shownProject(view)).toBe("All projects");
  });
});
