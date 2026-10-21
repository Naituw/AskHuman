import { defineComponent, h, ref, shallowRef, type Component } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import HistoryDetail from "./HistoryDetail.vue";
import { i18n } from "../i18n";
import type { HistoryEntry } from "../lib/types";
import { localImagePrepare, localImageReleaseScope, openPath, readImageDataUrl } from "../lib/ipc";
import { provideHistoryPreview, useHistoryPreview, type HistoryPreview } from "../views/history/useHistoryPreview";
import { startDrag } from "@crabnebula/tauri-plugin-drag";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: () => ({ close: vi.fn(async () => {}) }),
}));

vi.mock("@crabnebula/tauri-plugin-drag", () => ({
  startDrag: vi.fn(async () => {}),
}));

vi.mock("../lib/ipc", () => ({
  fileIconDataUrl: vi.fn(async () => ""),
  openPath: vi.fn(async () => {}),
  readImageDataUrl: vi.fn(async () => ""),
  localImageCreateScope: vi.fn(async () => "history-scope"),
  localImagePrepare: vi.fn(async (_scope: string, path: string) => ({ token: path, width: 10, height: 10 })),
  localImageReleaseAsset: vi.fn(async () => {}),
  localImageReleaseScope: vi.fn(async () => {}),
  showAttachmentMenu: vi.fn(async () => {}),
}));

function baseEntry(over: Partial<HistoryEntry>): HistoryEntry {
  return {
    id: "entry", timestampMs: 1, project: "", source: "", channel: "popup", action: "send", isMarkdown: false,
    message: { text: "", files: [] }, questions: [], answers: [], ...over,
  };
}

// The detail lives under HistoryView, which owns the window's single preview controller.
let mounted: VueWrapper[] = [];
function mountDetail(entry: HistoryEntry, stubs: Record<string, Component> = {}) {
  const current = shallowRef(entry);
  let preview!: HistoryPreview;
  const wrapper = mount(defineComponent({
    setup() {
      preview = provideHistoryPreview(useHistoryPreview({ entry: current, blocked: ref(false) }));
      return () => h(HistoryDetail, { entry: current.value, key: current.value.id });
    },
  }), { attachTo: document.body, global: { plugins: [i18n], stubs } });
  mounted.push(wrapper);
  return { wrapper, preview, current };
}
// `trigger` cannot set `detail`, which is how a second click of a double click is told apart.
function click(target: Element, detail = 1) {
  target.dispatchEvent(new MouseEvent("click", { bubbles: true, detail }));
}

describe("HistoryDetail", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    document.body.replaceChildren();
    i18n.global.locale.value = "en";
    mocks.invoke.mockImplementation(async (_command: string, args: { open?: boolean; version?: number }) => ({
      revision: args.version, side: args.open ? "right" : "closed", mainWidth: 600, mainHeight: 620,
    }));
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  });
  afterEach(() => { for (const wrapper of mounted) wrapper.unmount(); mounted = []; document.body.replaceChildren(); });

  it("shows the prompt and predefined options for a cancelled request", () => {
    const entry: HistoryEntry = baseEntry({
      id: "cancelled-request",
      timestampMs: 1_700_000_000_000,
      action: "cancel",
      questions: [
        {
          message: "Which release channel should we use?",
          predefinedOptions: [
            { text: "Stable", recommended: true },
            { text: "Beta", recommended: false },
          ],
        },
      ],
    });

    const { wrapper } = mountDetail(entry);

    expect(wrapper.find(".cancelled-note").exists()).toBe(false);
    expect(wrapper.find(".q-block").isVisible()).toBe(true);
    expect(wrapper.find(".plain-body").text()).toBe(
      "Which release channel should we use?"
    );
    expect(wrapper.findAll(".option").map((option) => option.text())).toEqual([
      "RecommendedStable",
      "Beta",
    ]);
    expect(wrapper.find(".rec-badge").text()).toBe("Recommended");
    expect(wrapper.find(".unanswered").text()).toBe("Not answered");
    wrapper.unmount();
  });

  it("routes stored Markdown prompts through the shared Mermaid component", () => {
    const entry: HistoryEntry = baseEntry({
      id: "mermaid-history",
      timestampMs: 1_700_000_000_000,
      isMarkdown: true,
      message: {
        text: "```mermaid\nsequenceDiagram\nA->>B: Hello\n```",
        files: [],
      },
    });

    const { wrapper } = mountDetail(entry, {
      MarkdownContent: {
        props: ["source"],
        template: '<div class="markdown-stub">{{ source }}</div>',
      },
    });

    expect(wrapper.get(".markdown-stub").text()).toContain("sequenceDiagram");
    wrapper.unmount();
  });

  it("uses display URLs for thumbnails; one click expands the panel, double click and Space only toggle it", async () => {
    const entry = baseEntry({
      id: "images", message: { text: "image", files: [{ path: "/tmp/a.png", name: "a.png", size: 10, isImage: true }] },
    });
    const { wrapper, preview } = mountDetail(entry);
    await flushPromises();
    expect(wrapper.get(".att-icon img").attributes("src")).toBe("askhuman-image://localhost//tmp/a.png");
    expect(readImageDataUrl).not.toHaveBeenCalled();
    expect(localImagePrepare).toHaveBeenCalledWith("history-scope", "/tmp/a.png");

    const attachment = wrapper.get(".attachment");
    expect(attachment.attributes("data-attachment-index")).toBe("0");
    click(attachment.element);
    await flushPromises();
    expect(preview.attach.selectedFile.value).toBe(0);
    expect(wrapper.get(".attachment").classes()).toContain("selected");
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_layout", expect.objectContaining({ requestId: "history:images", open: true }));

    // The second click and the dblclick of a double click change nothing and never open the system app.
    click(attachment.element, 2);
    await attachment.trigger("dblclick");
    expect(preview.attach.selectedFile.value).toBe(0);
    expect(openPath).not.toHaveBeenCalled();

    await attachment.trigger("keydown", { key: " " });
    await flushPromises();
    expect(preview.attach.selectedFile.value).toBeNull();
    expect(wrapper.get(".attachment").classes()).not.toContain("selected");
    expect(mocks.invoke).toHaveBeenLastCalledWith("popup_preview_layout", expect.objectContaining({ requestId: "history:images", open: false }));
    expect(openPath).not.toHaveBeenCalled();

    wrapper.unmount();
    await flushPromises();
    expect(localImageReleaseScope).toHaveBeenCalledWith("history-scope");
  });

  it("keeps the original-file actions: Enter opens it, the context menu and drag-out act on the history file", async () => {
    const entry = baseEntry({
      id: "files", message: { text: "files", files: [{ path: "/tmp/a.png", name: "a.png", size: 10, isImage: true }] },
    });
    const { wrapper } = mountDetail(entry);
    await flushPromises();
    const attachment = wrapper.get(".attachment");
    await attachment.trigger("keydown", { key: "Enter" });
    expect(openPath).toHaveBeenCalledExactlyOnceWith("/tmp/a.png");
    await attachment.trigger("contextmenu");
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_menu", { requestId: "history:files", index: 0 });
    await attachment.trigger("dragstart");
    const drag = vi.mocked(startDrag).mock.calls[0][0];
    expect(drag.item).toEqual(["/tmp/a.png"]);
    expect(drag.icon).toMatch(/^data:image\/png;base64,/);
    wrapper.unmount();
  });

  it("opens reply images in the panel like the popup does, while reply files still open in the system", async () => {
    const entry = baseEntry({
      id: "replies",
      questions: [{ message: "Pick", predefinedOptions: [] }],
      answers: [{ selectedOptions: [], userInput: "see attached", images: ["/tmp/x.png", "/tmp/y.png"], files: ["/tmp/notes.txt"] }],
    });
    const { wrapper, preview } = mountDetail(entry);
    await flushPromises();
    const thumbs = wrapper.findAll(".thumb-preview");
    expect(thumbs).toHaveLength(2);
    expect(thumbs[1].attributes("data-question-index")).toBe("0");
    expect(thumbs[1].attributes("data-image-index")).toBe("1");
    click(thumbs[1].element);
    await flushPromises();
    expect(preview.attach.isReplyPreview.value).toBe(true);
    expect(preview.attach.previewName.value).toBe("y.png");
    expect(wrapper.findAll(".thumb")[1].classes()).toContain("selected");
    expect(wrapper.findAll(".thumb")[0].classes()).not.toContain("selected");
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_layout", expect.objectContaining({ requestId: "history:replies", open: true }));
    expect(openPath).not.toHaveBeenCalled();

    click(thumbs[1].element);
    await flushPromises();
    expect(preview.attach.previewOpen.value).toBe(false);

    await wrapper.get(".reply-file").trigger("click");
    expect(openPath).toHaveBeenCalledExactlyOnceWith("/tmp/notes.txt");
    wrapper.unmount();
  });

  it("follows the entry on screen: switching entries collapses the panel and the new detail starts closed", async () => {
    const first = baseEntry({ id: "one", message: { text: "", files: [{ path: "/tmp/a.md", name: "a.md", size: 1, isImage: false }] } });
    const second = baseEntry({ id: "two", message: { text: "", files: [{ path: "/tmp/b.md", name: "b.md", size: 1, isImage: false }] } });
    const { wrapper, preview, current } = mountDetail(first);
    await flushPromises();
    click(wrapper.get(".attachment").element);
    await flushPromises();
    expect(preview.attach.previewOpen.value).toBe(true);
    current.value = second;
    await flushPromises();
    expect(preview.attach.previewOpen.value).toBe(false);
    expect(preview.ownerId.value).toBe("history:two");
    expect(wrapper.get(".att-name").text()).toBe("b.md");
    expect(wrapper.get(".attachment").classes()).not.toContain("selected");
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_prepare", expect.objectContaining({ requestId: "history:one", open: false }));
  });

  it("offers the original file when opening in the browser failed for a file the panel is not showing", async () => {
    const entry = baseEntry({ id: "browser", message: { text: "", files: [{ path: "/tmp/a.md", name: "a.md", size: 1, isImage: false }] } });
    const { wrapper, preview } = mountDetail(entry);
    await flushPromises();
    expect(wrapper.find(".attachment-browser-error").exists()).toBe(false);
    preview.attach.browserErrorIndex.value = 0;
    preview.attach.browserActionError.value = "limit";
    await flushPromises();
    const notice = wrapper.get(".attachment-browser-error");
    expect(notice.text()).toContain("a.md");
    await notice.get("button").trigger("click");
    expect(openPath).toHaveBeenCalledExactlyOnceWith("/tmp/a.md");
    wrapper.unmount();
  });
});
