import { enableAutoUnmount, flushPromises, mount } from "@vue/test-utils";
import { ref, shallowRef } from "vue";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import { openPath } from "../../lib/ipc";
import type { FileAttachment } from "../../lib/types";
import { AttachmentPanelKey, type AttachmentPanelContext } from "./attachmentPanelContext";
import { newReadingState } from "./useAttachmentContent";
import AttachmentPreviewPanel from "./AttachmentPreviewPanel.vue";

vi.mock("../../lib/ipc", () => ({ openPath: vi.fn(async () => {}) }));
// The native view and in-page find talk to the backend; this suite is about what the panel does with links.
vi.mock("./useNativeAttachmentPreview", async () => {
  const { ref } = await import("vue");
  return { useNativeAttachmentPreview: () => ({ nativeFailed: ref(false), syncNativePreview: vi.fn(async () => {}) }) };
});
vi.mock("./useAttachmentFind", async () => {
  const { ref } = await import("vue");
  return { useAttachmentFind: () => ({ adapter: {}, diffQuery: ref(""), diffCaseSensitive: ref(false), diffCurrent: ref(null) }) };
});

enableAutoUnmount(afterEach);

const guide: FileAttachment = { path: "/project/docs/guide.md", name: "guide.md", size: 120, isImage: false };

const html = [
  '<p><a href="" data-local-link="sibling.md">sibling</a>',
  '<a href="" data-local-link="../notes/a%20b.md:3">notes</a>',
  '<a href="" data-local-link="/abs/chart.png">abs</a>',
  '<a href="#sec">top</a>',
  '<a href="https://example.com/">web</a>',
  '<a href="">dead</a></p><h2 id="sec">Sec</h2>',
].join("");

function setup(overrides: Record<string, unknown> = {}) {
  const calls = {
    activateLink: vi.fn(), showLinkMenu: vi.fn(), goBack: vi.fn(), showPreview: vi.fn(), stopPreview: vi.fn(),
  };
  const context = {
    previewFile: ref<FileAttachment | null>(guide), previewIndex: ref<number | null>(2), previewName: ref("guide.md"),
    previewCount: ref(3), previewKey: ref("a:2"), isReplyPreview: ref(false),
    showPreview: calls.showPreview, stopPreview: calls.stopPreview, openFile: vi.fn(), openPreviewFile: vi.fn(),
    primaryBrowser: ref(false), browserOpening: ref(false), browserActionError: ref(null), browserErrorIndex: ref(null),
    showPreviewMenu: vi.fn(), revealFile: vi.fn(),
    popupActive: ref(true), panelContent: shallowRef({ kind: "markdown", text: "", html }), panelLoading: ref(false),
    panelReadingState: ref(newReadingState()), readingState: () => newReadingState(),
    onAttachmentDragStart: vi.fn(), previewActionError: ref(false), ownerId: ref("a"), nativePreviewBlocked: ref(false),
    previewTransition: ref(null), previewLayout: ref({ revision: 1 }), submitWithBareEnter: ref(false),
    registerNativePreviewSync: vi.fn(), findSupported: false, findActive: ref(false), findScope: ref("attachment"),
    openFind: vi.fn(), refreshFind: vi.fn(), registerAttachmentFind: vi.fn(),
    isLinkedPreview: ref(false), canGoBack: ref(false), goBack: calls.goBack,
    activateLink: calls.activateLink, showLinkMenu: calls.showLinkMenu, activeLinkPaths: ref(new Set<string>()),
    ...overrides,
  } as unknown as AttachmentPanelContext;
  const wrapper = mount(AttachmentPreviewPanel, {
    global: { plugins: [i18n], provide: { [AttachmentPanelKey as symbol]: context } }, attachTo: document.body,
  });
  return { wrapper, context, ...calls };
}
const mouse = (type: string, init: MouseEventInit = {}) => new MouseEvent(type, { bubbles: true, cancelable: true, ...init });
const anchors = (wrapper: ReturnType<typeof setup>["wrapper"]) => wrapper.findAll(".attachment-preview-markdown a");

describe("AttachmentPreviewPanel and the files the text links to", () => {
  beforeAll(async () => { await import("../../lib/markdown"); });
  beforeEach(() => {
    vi.mocked(openPath).mockClear();
    i18n.global.locale.value = "en";
    Element.prototype.scrollIntoView = vi.fn();
    vi.stubGlobal("CSS", { escape: (value: string) => value });
  });
  afterEach(() => vi.unstubAllGlobals());

  it("steps through attachments with a count and arrows", async () => {
    const { wrapper, showPreview } = setup();
    await flushPromises();
    expect(wrapper.get(".attachment-preview-count").text()).toBe("3 / 3");
    const prev = wrapper.get(`button[aria-label="${i18n.global.t("popup.prev")}"]`);
    const next = wrapper.get(`button[aria-label="${i18n.global.t("popup.next")}"]`);
    expect(next.attributes("disabled")).toBeDefined();
    await prev.trigger("click");
    expect(showPreview).toHaveBeenCalledWith(1);
    expect(wrapper.find(".attachment-preview-back").exists()).toBe(false);
  });

  it("shows a linked file on its own: no count, no arrows, the same close, open and menu", async () => {
    const { wrapper, stopPreview } = setup({ isLinkedPreview: ref(true) });
    await flushPromises();
    expect(wrapper.find(".attachment-preview-count").exists()).toBe(false);
    expect(wrapper.find(`button[aria-label="${i18n.global.t("popup.prev")}"]`).exists()).toBe(false);
    expect(wrapper.find(`button[aria-label="${i18n.global.t("popup.next")}"]`).exists()).toBe(false);
    expect(wrapper.find(".attachment-preview-open").exists()).toBe(true);
    expect(wrapper.find(`button[aria-label="${i18n.global.t("popup.preview.more")}"]`).exists()).toBe(true);
    await wrapper.get(`button[aria-label="${i18n.global.t("popup.preview.close")}"]`).trigger("click");
    expect(stopPreview).toHaveBeenCalled();
  });

  it("offers Back only while there is somewhere to go back to", async () => {
    const canGoBack = ref(false);
    const { wrapper, goBack } = setup({ isLinkedPreview: ref(true), canGoBack });
    await flushPromises();
    expect(wrapper.find(".attachment-preview-back").exists()).toBe(false);
    canGoBack.value = true;
    await flushPromises();
    const back = wrapper.get(".attachment-preview-back");
    expect(back.text()).toContain(i18n.global.t("popup.preview.back"));
    expect(back.attributes("aria-label")).toBe(i18n.global.t("popup.preview.back"));
    await back.trigger("click");
    expect(goBack).toHaveBeenCalledTimes(1);
  });

  it("opens a link in the previewed Markdown file relative to that file, as a jump inside the panel", async () => {
    const { wrapper, activateLink } = setup();
    await flushPromises();
    const [sibling, notes, abs] = anchors(wrapper);
    const click = mouse("click", { metaKey: true });
    sibling.element.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    expect(activateLink).toHaveBeenCalledExactlyOnceWith("/project/docs/sibling.md", click, true);
    notes.element.dispatchEvent(mouse("click"));
    // Folding `..` is the backend's job when it registers the path.
    expect(activateLink).toHaveBeenLastCalledWith("/project/docs/../notes/a b.md", expect.any(MouseEvent), true);
    abs.element.dispatchEvent(mouse("click"));
    expect(activateLink).toHaveBeenLastCalledWith("/abs/chart.png", expect.any(MouseEvent), true);
    expect(openPath).not.toHaveBeenCalled();
  });

  it("resolves a link relative to a Windows file, whatever prefix the canonical path carries", async () => {
    const { wrapper, activateLink } = setup({
      previewFile: ref<FileAttachment>({ path: "\\\\?\\C:\\project\\docs\\guide.md", name: "guide.md", size: 1, isImage: false }),
    });
    await flushPromises();
    anchors(wrapper)[0].element.dispatchEvent(mouse("click"));
    // The verbatim prefix does not survive into the base, and the file's own separator style does.
    expect(activateLink).toHaveBeenCalledExactlyOnceWith("C:\\project\\docs\\sibling.md", expect.any(MouseEvent), true);
  });

  it("keeps in-page anchors, web links and dead links as they were", async () => {
    const { wrapper, activateLink } = setup();
    await flushPromises();
    const [, , , top, web, dead] = anchors(wrapper);
    const anchor = mouse("click");
    top.element.dispatchEvent(anchor);
    expect(anchor.defaultPrevented).toBe(true);
    expect(Element.prototype.scrollIntoView).toHaveBeenCalledTimes(1);
    web.element.dispatchEvent(mouse("click"));
    expect(openPath).toHaveBeenCalledExactlyOnceWith("https://example.com/");
    const inert = mouse("click");
    dead.element.dispatchEvent(inert);
    expect(inert.defaultPrevented).toBe(true);
    expect(activateLink).not.toHaveBeenCalled();
  });

  it("does nothing with a link while no file is on show", async () => {
    const { wrapper, activateLink } = setup({ previewFile: ref(null) });
    await flushPromises();
    const click = mouse("click");
    anchors(wrapper)[0].element.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    expect(activateLink).not.toHaveBeenCalled();
  });

  it("opens the same menu on a right click of a local link, and leaves everything else to the system", async () => {
    const { wrapper, showLinkMenu } = setup();
    await flushPromises();
    const [sibling, , , top, web] = anchors(wrapper);
    const right = mouse("contextmenu");
    sibling.element.dispatchEvent(right);
    expect(right.defaultPrevented).toBe(true);
    expect(showLinkMenu).toHaveBeenCalledExactlyOnceWith("/project/docs/sibling.md");
    for (const other of [top, web]) {
      const event = mouse("contextmenu");
      other.element.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(false);
    }
    const text = mouse("contextmenu");
    wrapper.get(".attachment-preview-markdown h2").element.dispatchEvent(text);
    expect(text.defaultPrevented).toBe(false);
    expect(showLinkMenu).toHaveBeenCalledTimes(1);
  });
});
