import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../i18n";
import TodosView from "./TodosView.vue";
import { ALL_KEY } from "./todos/todoProjects";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

const ipc = vi.hoisted(() => ({
  openNewTask: vi.fn(async () => {}),
  openPath: vi.fn(async (_path?: string) => {}),
  readImageDataUrl: vi.fn(async () => "data:image/png;base64,cGF0aA=="),
  localImageCreateScope: vi.fn(async () => "todo-scope"),
  localImagePrepare: vi.fn(async (_scope: string, path: string) => ({ token: path, width: 10, height: 10 })),
  localImageReleaseAsset: vi.fn(async () => {}),
  localImageReleaseScope: vi.fn(async () => {}),
  openSettings: vi.fn(async (_tab?: string) => {}),
  todoSyncStatus: vi.fn(async (): Promise<Record<string, unknown>> => ({ supported: false, enabled: false })),
  todoSyncTouch: vi.fn(async () => {}),
  todoSyncHistoryCount: vi.fn(async (_project?: string): Promise<number | null> => null),
  todoSyncResume: vi.fn(async () => ({})),
  todosAdd: vi.fn(async () => ({ id: "created" })),
  todosClear: vi.fn(async () => {}),
  todosAllPending: vi.fn(async (): Promise<unknown[]> => []),
  todosComplete: vi.fn(async (_project?: string, _id?: string) => {}),
  todosHistory: vi.fn(async (_project?: string): Promise<unknown[]> => []),
  todosHistoryClear: vi.fn(async () => {}),
  todosInit: vi.fn(async (): Promise<Record<string, unknown>> => ({
    theme: "system",
    lang: "zh",
    popupSubmitKey: "cmdEnter",
    newTaskSupported: false,
  })),
  todosList: vi.fn(async (_project?: string): Promise<unknown[]> => []),
  todosProjects: vi.fn(async (): Promise<unknown[]> => [
    { key: "/project", name: "project", count: 0, section: "recent" },
  ]),
  todosProjectsEnriched: vi.fn(async (): Promise<unknown[]> => [
    { key: "/project", name: "project", count: 0, section: "recent" },
  ]),
  todosRemove: vi.fn(async () => {}),
  todosReorder: vi.fn(async () => {}),
  todosRestore: vi.fn(async () => {}),
  todosSetAuto: vi.fn(async () => {}),
  todosAttachPastedImages: vi.fn(async () => ({ id: "updated" })),
  todosUpdate: vi.fn(async () => ({ id: "updated" })),
  todosUpdateAttachments: vi.fn(async () => ({ id: "updated" })),
  fileIconDataUrl: vi.fn(async () => null),
  previewAttachments: vi.fn(async () => {}),
  showAttachmentMenu: vi.fn(async () => {}),
  todoAttachmentThumbnail: vi.fn(async () => null),
}));

const tauriEvents = vi.hoisted(() => ({
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("../lib/ipc", () => ipc);
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
}));
vi.mock("../lib/theme", () => ({
  applyTheme: vi.fn(),
  fileToDataUrl: vi.fn(async () => "data:image/png;base64,cG5n"),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    async (name: string, handler: (event: { payload: unknown }) => void) => {
      tauriEvents.listeners.set(name, handler);
      return () => tauriEvents.listeners.delete(name);
    }
  ),
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: vi.fn(async () => vi.fn()) }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async (): Promise<string[] | null> => null) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({
  startDrag: vi.fn(async () => {}),
}));

function pasteEvent(items: DataTransferItemList): Event {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", { value: { items } });
  return event;
}

describe("TodosView new-todo clipboard attachments", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriEvents.listeners.clear();
    i18n.global.locale.value = "zh";
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("keeps text paste native and submits pasted images with the new todo", async () => {
    const wrapper = mount(TodosView, {
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    await flushPromises();
    const input = wrapper.get<HTMLTextAreaElement>(".td-input");

    const textItems = {
      0: { kind: "string", type: "text/plain", getAsFile: () => null },
      length: 1,
    } as unknown as DataTransferItemList;
    const textPaste = pasteEvent(textItems);
    input.element.dispatchEvent(textPaste);
    expect(textPaste.defaultPrevented).toBe(false);

    const png = new File(["png"], "capture.png", { type: "image/png" });
    const imageItems = {
      0: { kind: "file", type: "image/png", getAsFile: () => png },
      length: 1,
    } as unknown as DataTransferItemList;
    const imagePaste = pasteEvent(imageItems);
    input.element.dispatchEvent(imagePaste);
    await flushPromises();

    expect(imagePaste.defaultPrevented).toBe(true);
    expect(wrapper.get(".thumb").attributes("title")).toBe("capture.png");
    expect(wrapper.get(".thumb img").attributes("src")).toBe(
      "data:image/png;base64,cG5n"
    );
    await wrapper.get(".thumb .remove").trigger("click");
    expect(wrapper.find(".thumb").exists()).toBe(false);

    input.element.dispatchEvent(pasteEvent(imageItems));
    await flushPromises();

    await input.setValue("Create from clipboard");
    await wrapper.get(".td-btn-add").trigger("click");
    await flushPromises();

    expect(ipc.todosAdd).toHaveBeenCalledWith(
      "/project",
      "Create from clipboard",
      false,
      [],
      [
        {
          data: "data:image/png;base64,cG5n",
          mediaType: "image/png",
          filename: "capture.png",
        },
      ]
    );
    expect(wrapper.find(".thumb").exists()).toBe(false);
    expect(input.element.value).toBe("");
    wrapper.unmount();
  });

  it("requires a second click before removing a stored attachment", async () => {
    ipc.todosList.mockResolvedValueOnce([
      {
        id: "todo-1",
        text: "Keep this attachment safe",
        createdAtMs: Date.now(),
        attachments: [
          {
            id: "attachment-1",
            name: "capture.png",
            size: 1024,
            isImage: true,
            sourcePath: "/source/capture.png",
            path: "/managed/capture.png",
            storage: "managed",
            available: true,
          },
        ],
      },
    ]);
    const wrapper = mount(TodosView, {
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    await flushPromises();

    await wrapper.get(".todo-attachments-summary").trigger("click");
    await wrapper.get(".todo-attachment-remove").trigger("click");
    expect(ipc.todosUpdateAttachments).not.toHaveBeenCalled();
    expect(wrapper.get(".todo-attachment-remove-confirm").text()).toBe("确认删除");

    await wrapper.get(".todo-attachment-remove-confirm").trigger("click");
    await flushPromises();
    expect(ipc.todosUpdateAttachments).toHaveBeenCalledWith(
      "/project",
      "todo-1",
      [],
      ["attachment-1"]
    );
    wrapper.unmount();
  });

  it("submits selected image paths and retains a file chip after a display decoding error", async () => {
    vi.mocked(openDialog).mockResolvedValueOnce(["/tmp/a.png", "/tmp/b.png"]);
    const wrapper = mount(TodosView, { attachTo: document.body, global: { plugins: [i18n] } });
    await flushPromises();
    await wrapper.get(".td-attach-picker").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".thumb img").map(img => img.attributes("src"))).toEqual([
      "askhuman-image://localhost//tmp/a.png", "askhuman-image://localhost//tmp/b.png",
    ]);
    expect(ipc.readImageDataUrl).not.toHaveBeenCalled();
    await wrapper.findAll(".thumb img")[0].trigger("error");
    await flushPromises();
    expect(wrapper.findAll(".thumb")).toHaveLength(1);
    expect(wrapper.get(".reply-files").text()).toContain("a.png");
    expect(ipc.localImageReleaseAsset).toHaveBeenCalledWith("todo-scope", "/tmp/a.png");
    await wrapper.get(".td-input").setValue("Use original paths");
    await wrapper.get(".td-btn-add").trigger("click");
    await flushPromises();
    expect(ipc.todosAdd).toHaveBeenCalledWith("/project", "Use original paths", false, ["/tmp/a.png", "/tmp/b.png"], []);
    expect(ipc.localImageReleaseScope).toHaveBeenCalledWith("todo-scope");
    wrapper.unmount();
  });

  it("syncs the attachment highlight from the window-level Quick Look event", async () => {
    const storedAttachment = (id: string) => ({
      id,
      name: `${id}.pdf`,
      size: 1024,
      isImage: false,
      sourcePath: `/source/${id}.pdf`,
      path: `/managed/${id}.pdf`,
      storage: "managed",
      available: true,
    });
    ipc.todosList.mockResolvedValueOnce([
      {
        id: "todo-preview",
        text: "Preview attachments",
        createdAtMs: Date.now(),
        attachments: [storedAttachment("first"), storedAttachment("second")],
      },
    ]);
    const wrapper = mount(TodosView, {
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    await flushPromises();
    await wrapper.get(".todo-attachments-summary").trigger("click");
    const rows = wrapper.findAll(".todo-attachment");
    await rows[0].trigger("click");
    await rows[0].trigger("keydown", { key: " " });

    expect(ipc.previewAttachments).toHaveBeenCalledWith(
      ["/managed/first.pdf", "/managed/second.pdf"],
      0
    );
    tauriEvents.listeners.get("preview-index")?.({ payload: 1 });
    await flushPromises();

    expect(wrapper.findAll(".todo-attachment")[0].classes()).not.toContain("selected");
    expect(wrapper.findAll(".todo-attachment")[1].classes()).toContain("selected");
    wrapper.unmount();
  });
});

// ===== Sidebar navigation and the "All todos" view (spec todo-sync §9) =====

const A = "/Users/me/a";
const B = "/Users/me/b";
const C = "/Users/me/c";

function todo(id: string, text: string) {
  return { id, text, createdAtMs: Date.now() - 60_000 };
}

/** Two projects with todos (a: 2, b: 1) and one recent project; the home directory is /Users/me. */
function useWorkspace(): void {
  const projects = [
    { key: A, name: "a", count: 2, section: "withTodos" },
    { key: B, name: "b", count: 1, section: "withTodos" },
    { key: C, name: "c", count: 0, section: "recent" },
  ];
  ipc.todosProjects.mockImplementation(async () => projects);
  ipc.todosProjectsEnriched.mockImplementation(async () => projects);
  ipc.todosList.mockImplementation(async (project?: string) =>
    project === A
      ? [todo("a1", "alpha one"), todo("a2", "alpha two")]
      : project === B
        ? [todo("b1", "bravo one")]
        : []
  );
  // Deliberately not in sidebar order: the view must order the groups like the sidebar.
  ipc.todosAllPending.mockImplementation(async () => [
    { key: B, entries: [todo("b1", "bravo one")] },
    { key: A, entries: [todo("a1", "alpha one"), todo("a2", "alpha two")] },
  ]);
  ipc.todosInit.mockImplementation(async () => ({
    theme: "system",
    lang: "zh",
    popupSubmitKey: "cmdEnter",
    newTaskSupported: false,
    home: "/Users/me",
  }));
}

describe("TodosView sidebar", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriEvents.listeners.clear();
    i18n.global.locale.value = "zh";
    // jsdom has no layout, hence no scrollIntoView.
    Element.prototype.scrollIntoView = vi.fn();
    useWorkspace();
  });

  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  async function open() {
    const wrapper = mount(TodosView, { attachTo: document.body, global: { plugins: [i18n] } });
    await flushPromises();
    return wrapper;
  }

  const rowTexts = (wrapper: Awaited<ReturnType<typeof open>>) =>
    wrapper.findAll(".td-row .td-text").map((node) => node.text());

  it("lists the projects in the sidebar and describes the selected one in the header", async () => {
    const wrapper = await open();
    expect(
      wrapper.findAll("[role=option]").map((node) => node.attributes("data-key"))
    ).toEqual([ALL_KEY, A, B, C]);
    expect(wrapper.get(".td-project").text()).toBe("a");
    expect(wrapper.get(".td-path").text()).toBe("~/a");
    expect(rowTexts(wrapper)).toEqual(["alpha one", "alpha two"]);
    expect(wrapper.find("select").exists()).toBe(false);
    wrapper.unmount();
  });

  it("switches the todo list when another project is picked", async () => {
    const wrapper = await open();
    await wrapper.get(`[data-key="${B}"]`).trigger("click");
    await flushPromises();
    expect(ipc.todosList).toHaveBeenCalledWith(B);
    expect(wrapper.get(".td-project").text()).toBe("b");
    expect(rowTexts(wrapper)).toEqual(["bravo one"]);
    expect(wrapper.get(`[data-key="${B}"]`).attributes("aria-selected")).toBe("true");
    wrapper.unmount();
  });

  it("narrows the sidebar with the search box", async () => {
    const wrapper = await open();
    await wrapper.get(".td-side-input").setValue("b");
    expect(
      wrapper.findAll("[role=option]").map((node) => node.attributes("data-key"))
    ).toEqual([ALL_KEY, B]);
    wrapper.unmount();
  });

  it("preselects the project of the todos-goto-project event", async () => {
    const wrapper = await open();
    tauriEvents.listeners.get("todos-goto-project")?.({ payload: C });
    await flushPromises();
    expect(wrapper.get(".td-project").text()).toBe("c");
    expect(wrapper.get(`[data-key="${C}"]`).attributes("aria-selected")).toBe("true");
    wrapper.unmount();
  });

  it("shows every project's pending todos in All, ordered like the sidebar", async () => {
    const wrapper = await open();
    await wrapper.get(`[data-key="${ALL_KEY}"]`).trigger("click");
    await flushPromises();
    expect(ipc.todosAllPending).toHaveBeenCalledTimes(1);
    expect(wrapper.get(".td-project").text()).toBe("全部待办");
    expect(wrapper.get(".td-path").text()).toBe("2 个项目 · 3 项");
    expect(wrapper.findAll(".td-all-name").map((node) => node.text())).toEqual(["a", "b"]);
    expect(rowTexts(wrapper)).toEqual(["alpha one", "alpha two", "bravo one"]);
    // The aggregate has no add box and no project menu.
    expect(wrapper.find(".td-footer").exists()).toBe(false);
    expect(wrapper.find(".td-menu-wrap").exists()).toBe(false);
    wrapper.unmount();
  });

  it("reloads All when the todos change", async () => {
    const wrapper = await open();
    await wrapper.get(`[data-key="${ALL_KEY}"]`).trigger("click");
    await flushPromises();
    tauriEvents.listeners.get("todos-updated")?.({ payload: null });
    await flushPromises();
    expect(ipc.todosAllPending).toHaveBeenCalledTimes(2);
    wrapper.unmount();
  });

  it("completes a todo of another project from All after the undo window", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const wrapper = await open();
    await wrapper.get(`[data-key="${ALL_KEY}"]`).trigger("click");
    await flushPromises();
    await wrapper.findAll(".td-all-row .td-check")[2].trigger("click");
    expect(wrapper.findAll(".td-all-row")[2].classes()).toContain("completing");
    expect(ipc.todosComplete).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1000);
    await flushPromises();
    expect(ipc.todosComplete).toHaveBeenCalledWith(B, "b1");
    wrapper.unmount();
  });

  it("cancels a pending complete when it is unchecked within the undo window", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    const wrapper = await open();
    await wrapper.get(`[data-key="${ALL_KEY}"]`).trigger("click");
    await flushPromises();
    const check = wrapper.findAll(".td-all-row .td-check")[0];
    await check.trigger("click");
    await check.trigger("click");
    vi.advanceTimersByTime(2000);
    await flushPromises();
    expect(ipc.todosComplete).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it("opens a project from All with the clicked todo selected", async () => {
    const wrapper = await open();
    await wrapper.get(`[data-key="${ALL_KEY}"]`).trigger("click");
    await flushPromises();
    await wrapper.findAll(".td-all-row")[1].trigger("click");
    await flushPromises();
    expect(wrapper.get(".td-project").text()).toBe("a");
    expect(wrapper.find(".td-all").exists()).toBe(false);
    expect(wrapper.get(".td-row.selected .td-text").text()).toBe("alpha two");
    wrapper.unmount();
  });

  it("opens a project from its group header in All", async () => {
    const wrapper = await open();
    await wrapper.get(`[data-key="${ALL_KEY}"]`).trigger("click");
    await flushPromises();
    await wrapper.findAll(".td-all-open")[1].trigger("click");
    await flushPromises();
    expect(wrapper.get(".td-project").text()).toBe("b");
    expect(rowTexts(wrapper)).toEqual(["bravo one"]);
    wrapper.unmount();
  });

  it("collapses long aggregates behind a show-more button", async () => {
    ipc.todosAllPending.mockImplementation(async () => [
      {
        key: A,
        entries: Array.from({ length: 120 }, (_, i) => todo(`t${i}`, `item ${i}`)),
      },
    ]);
    const wrapper = await open();
    await wrapper.get(`[data-key="${ALL_KEY}"]`).trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".td-all-row")).toHaveLength(50);
    expect(wrapper.get(".td-all-count").text()).toBe("120 项");
    expect(wrapper.get(".td-all-more").text()).toBe("显示更多（70）");
    await wrapper.get(".td-all-more").trigger("click");
    expect(wrapper.findAll(".td-all-row")).toHaveLength(100);
    await wrapper.get(".td-all-more").trigger("click");
    expect(wrapper.findAll(".td-all-row")).toHaveLength(120);
    expect(wrapper.find(".td-all-more").exists()).toBe(false);
    wrapper.unmount();
  });

  it("says so when no project has a todo", async () => {
    ipc.todosAllPending.mockImplementation(async () => []);
    const wrapper = await open();
    await wrapper.get(`[data-key="${ALL_KEY}"]`).trigger("click");
    await flushPromises();
    expect(wrapper.get(".empty-title").text()).toBe("还没有待办");
    wrapper.unmount();
  });

  it("shows the sidebar as a drawer in narrow windows and closes it on pick", async () => {
    const wrapper = await open();
    const root = wrapper.get(".todos-win");
    expect(root.classes()).not.toContain("drawer-open");
    await wrapper.get(".td-side-toggle").trigger("click");
    expect(root.classes()).toContain("drawer-open");
    expect(wrapper.find(".td-scrim").exists()).toBe(true);
    await wrapper.get(".td-scrim").trigger("click");
    expect(root.classes()).not.toContain("drawer-open");
    await wrapper.get(".td-side-toggle").trigger("click");
    await wrapper.get(`[data-key="${B}"]`).trigger("click");
    await flushPromises();
    expect(root.classes()).not.toContain("drawer-open");
    wrapper.unmount();
  });

  it("jumps to the project search on the find shortcut", async () => {
    const wrapper = await open();
    window.dispatchEvent(
      // jsdom is never a Mac, so the primary modifier is Ctrl.
      new KeyboardEvent("keydown", { key: "f", ctrlKey: true, cancelable: true })
    );
    await flushPromises();
    expect(document.activeElement).toBe(wrapper.get(".td-side-input").element);
    wrapper.unmount();
  });

  it("copies the project path and reveals the folder from the project menu", async () => {
    const writeText = vi.fn(async (_text: string) => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const wrapper = await open();
    expect(wrapper.find(".td-menu").exists()).toBe(false);
    await wrapper.get(".td-menu-wrap .td-icon-btn").trigger("click");
    const items = wrapper.findAll(".td-menu-item");
    expect(items).toHaveLength(2);
    await items[0].trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith(A);
    expect(wrapper.find(".td-menu").exists()).toBe(false);

    await wrapper.get(".td-menu-wrap .td-icon-btn").trigger("click");
    await wrapper.findAll(".td-menu-item")[1].trigger("click");
    await flushPromises();
    expect(ipc.openPath).toHaveBeenCalledWith(A);
    wrapper.unmount();
  });
});


// ===== iCloud sync line and the global clear-history confirmation (spec todo-sync §9.6, §9.7, T16) =====

function syncStatus(patch: Record<string, unknown> = {}): Record<string, unknown> {
  const now = Date.now();
  return {
    supported: true,
    enabled: true,
    syncing: false,
    deviceName: "Studio",
    paused: null,
    lastPullOkMs: now - 40_000,
    lastPushOkMs: now - 100_000,
    unsynced: 0,
    todos: 3,
    history: 2,
    devices: [],
    helper: { state: "running", version: "1", environment: "production", problem: null, installed: true },
    lastTick: null,
    newerState: false,
    ...patch,
  };
}

describe("TodosView iCloud sync", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    tauriEvents.listeners.clear();
    i18n.global.locale.value = "zh";
    Element.prototype.scrollIntoView = vi.fn();
    useWorkspace();
    ipc.todosHistory.mockImplementation(async () => [
      { id: "h1", text: "done one", createdAtMs: 1, agentKind: null, doneAtMs: 2 },
      { id: "h2", text: "done two", createdAtMs: 1, agentKind: null, doneAtMs: 3 },
    ]);
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  /** Mount and wait past the two animation frames after which the status is requested. */
  async function open() {
    const wrapper = mount(TodosView, { attachTo: document.body, global: { plugins: [i18n] } });
    await flushPromises();
    await new Promise((resolve) => setTimeout(resolve, 80));
    await flushPromises();
    return wrapper;
  }

  it("shows no sync line where iCloud sync does not exist", async () => {
    const wrapper = await open();
    expect(ipc.todoSyncStatus).toHaveBeenCalledTimes(1);
    expect(wrapper.find('[data-testid="sync-footer"]').exists()).toBe(false);
    expect(wrapper.find(".td-side-foot").exists()).toBe(false);
    wrapper.unmount();
  });

  it("offers iCloud sync while the todos are local only, and opens its card in Settings", async () => {
    ipc.todoSyncStatus.mockResolvedValue(syncStatus({ enabled: false }));
    const wrapper = await open();
    const footer = wrapper.get('[data-testid="sync-footer"]');
    expect(footer.text()).toContain("仅本机");
    await footer.get(".td-sync-action").trigger("click");
    expect(ipc.openSettings).toHaveBeenCalledWith("experimental#todo-sync-card");
    wrapper.unmount();
  });

  it("says how long ago it synced, and nothing about it before the first frame", async () => {
    ipc.todoSyncStatus.mockResolvedValue(syncStatus());
    const wrapper = mount(TodosView, { attachTo: document.body, global: { plugins: [i18n] } });
    await flushPromises();
    // The data is on screen; the status has not been asked for yet (it waits for the first frame).
    expect(ipc.todosList).toHaveBeenCalled();
    expect(wrapper.find('[data-testid="sync-footer"]').exists()).toBe(false);
    await new Promise((resolve) => setTimeout(resolve, 80));
    await flushPromises();
    expect(wrapper.get('[data-testid="sync-footer"]').text()).toContain("已同步 · 40 秒前");
    wrapper.unmount();
  });

  it("retries a full iCloud from the line itself", async () => {
    ipc.todoSyncStatus.mockResolvedValue(syncStatus({ paused: "quota" }));
    const wrapper = await open();
    const footer = wrapper.get('[data-testid="sync-footer"]');
    expect(footer.text()).toContain("iCloud 空间不足");
    await footer.get(".td-sync-action").trigger("click");
    await flushPromises();
    expect(ipc.todoSyncResume).toHaveBeenCalledTimes(1);
    expect(ipc.openSettings).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  async function openHistoryDialog(wrapper: Awaited<ReturnType<typeof open>>) {
    await wrapper.get(".td-hist-toggle").trigger("click");
    await wrapper.get(".td-hist-head .td-clear").trigger("click");
    await flushPromises();
  }

  it("says clearing the history clears it on every Mac, with the cloud's count", async () => {
    ipc.todoSyncStatus.mockResolvedValue(syncStatus());
    ipc.todoSyncHistoryCount.mockResolvedValue(14);
    const wrapper = await open();
    await openHistoryDialog(wrapper);
    expect(ipc.todoSyncHistoryCount).toHaveBeenCalledWith(A);
    const dialog = wrapper.get(".td-dialog");
    expect(dialog.text()).toContain("所有 Mac 与 iCloud");
    expect(dialog.text()).toContain("共 14 条");
    await dialog.get(".td-btn-danger").trigger("click");
    await flushPromises();
    expect(ipc.todosHistoryClear).toHaveBeenCalledWith(A);
    wrapper.unmount();
  });

  it("keeps the local wording for a project that does not sync, or while sync is off", async () => {
    ipc.todoSyncStatus.mockResolvedValue(syncStatus());
    ipc.todoSyncHistoryCount.mockResolvedValue(null);
    let wrapper = await open();
    await openHistoryDialog(wrapper);
    expect(wrapper.get(".td-dialog").text()).toContain("将删除本项目的 2 条历史记录");
    wrapper.unmount();

    ipc.todoSyncStatus.mockResolvedValue(syncStatus({ enabled: false }));
    ipc.todoSyncHistoryCount.mockClear();
    wrapper = await open();
    await openHistoryDialog(wrapper);
    expect(ipc.todoSyncHistoryCount).not.toHaveBeenCalled();
    expect(wrapper.get(".td-dialog").text()).toContain("将删除本项目的 2 条历史记录");
    wrapper.unmount();
  });

  it("warns that clearing the todos of a synced project deletes them on the other Macs too", async () => {
    ipc.todoSyncStatus.mockResolvedValue(syncStatus());
    ipc.todoSyncHistoryCount.mockResolvedValue(5);
    const wrapper = await open();
    await wrapper.get(".td-clear-row .td-clear").trigger("click");
    await flushPromises();
    expect(wrapper.get(".td-dialog").text()).toContain("也会从其他 Mac 上删除");
    wrapper.unmount();
  });
});
