// The unsent content of a cancelled ask in the history detail (docs/specs/popup-draft-on-cancel.md D8): what the
// entry shows under each question, the line that says what it is, the copy buttons, and what stays exactly as it was.
import { defineComponent, h, ref, shallowRef } from "vue";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import HistoryDetail from "./HistoryDetail.vue";
import { i18n } from "../i18n";
import type { HistoryDraft, HistoryEntry } from "../lib/types";
import { openPath } from "../lib/ipc";
import { provideHistoryPreview, useHistoryPreview, type HistoryPreview } from "../views/history/useHistoryPreview";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
  invoke: mocks.invoke,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/webviewWindow", () => ({ getCurrentWebviewWindow: () => ({ close: vi.fn(async () => {}) }) }));
vi.mock("@crabnebula/tauri-plugin-drag", () => ({ startDrag: vi.fn(async () => {}) }));
vi.mock("../lib/ipc", () => ({
  fileIconDataUrl: vi.fn(async () => ""),
  attachmentsExist: vi.fn(async (paths: string[]) => paths.map(() => true)),
  openPath: vi.fn(async () => {}),
  readImageDataUrl: vi.fn(async () => ""),
  localImageCreateScope: vi.fn(async () => "history-scope"),
  localImagePrepare: vi.fn(async (_scope: string, path: string) => ({ token: path, width: 10, height: 10 })),
  localImageReleaseAsset: vi.fn(async () => {}),
  localImageReleaseScope: vi.fn(async () => {}),
  showAttachmentMenu: vi.fn(async () => {}),
}));

const QUESTIONS = [
  { message: "Which one should I take?", predefinedOptions: [{ text: "Yes", recommended: false }, { text: "No", recommended: false }] },
  { message: "Anything else?", predefinedOptions: [] },
];

function draft(over: Partial<HistoryDraft> = {}): HistoryDraft {
  return {
    answers: [
      { selectedOptions: ["No"], userInput: "because it is slow", images: ["/tmp/askhuman/gone/shot.png"], files: ["/tmp/dir/report.txt"] },
      { selectedOptions: [], userInput: "", images: [], files: [] },
    ],
    annotations: [{ quote: "the plan", note: "which plan?" }],
    ...over,
  };
}

function cancelled(over: Partial<HistoryEntry> = {}): HistoryEntry {
  return {
    id: "gone", timestampMs: 1_700_000_000_000, project: "", source: "", channel: "caller", action: "cancel", isMarkdown: false,
    message: { text: "Review the plan", files: [] }, questions: QUESTIONS, answers: [], draft: draft(), ...over,
  };
}

let mounted: VueWrapper[] = [];
function mountDetail(entry: HistoryEntry) {
  let preview!: HistoryPreview;
  const wrapper = mount(defineComponent({
    setup() {
      preview = provideHistoryPreview(useHistoryPreview({ entry: shallowRef(entry), blocked: ref(false) }));
      return () => h(HistoryDetail, { entry, key: entry.id });
    },
  }), { attachTo: document.body, global: { plugins: [i18n] } });
  mounted.push(wrapper);
  return { wrapper, preview };
}
function click(target: Element) {
  target.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
}

describe("the unsent content of a cancelled ask in the detail", () => {
  let copied: string[];
  beforeEach(() => {
    vi.clearAllMocks();
    document.body.replaceChildren();
    i18n.global.locale.value = "en";
    copied = [];
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn(async (text: string) => { copied.push(text); }) } });
    mocks.invoke.mockImplementation(async (_command: string, args: { open?: boolean; version?: number }) => ({
      revision: args.version, side: args.open ? "right" : "closed", mainWidth: 600, mainHeight: 620,
    }));
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  });
  afterEach(() => { for (const wrapper of mounted) wrapper.unmount(); mounted = []; document.body.replaceChildren(); });

  it("says what it is, and whether a new question has taken it", async () => {
    const { wrapper } = mountDetail(cancelled());
    expect(wrapper.get(".draft-line").text()).toContain("Not submitted when the caller went away");
    expect(wrapper.get(".draft-line-state").text()).toBe("Not loaded into a new question");
    expect(wrapper.get(".draft-line-state").classes()).not.toContain("loaded");
    // The record is still a cancellation: the banner and "Not answered" are as they were.
    expect(wrapper.get(".status-banner").text()).toContain("Cancelled via Caller");
    expect(wrapper.findAll(".unanswered").map((line) => line.text())).toEqual(["Not answered", "Not answered"]);

    const loaded = mountDetail(cancelled({ id: "taken", draft: draft({ claimedBy: "request-2" }) }));
    expect(loaded.wrapper.get(".draft-line-state").text()).toBe("Loaded into a new question");
    expect(loaded.wrapper.get(".draft-line-state").classes()).toContain("loaded");
  });

  it("shows each question's own content under it, read-only, and nothing under a question that has none", async () => {
    const { wrapper } = mountDetail(cancelled());
    await flushPromises();
    const boxes = wrapper.findAll(".draft-block");
    expect(boxes).toHaveLength(1);
    const blocks = wrapper.findAll(".q-block");
    expect(blocks[0].find(".draft-block").exists()).toBe(true);
    expect(blocks[1].find(".draft-block").exists()).toBe(false);
    const box = boxes[0];
    expect(box.find(".draft-tag").text()).toBe("Draft");
    expect(box.findAll(".draft-option").map((option) => option.text())).toEqual(["✓ No"]);
    expect(box.get(".reply-text").text()).toBe("because it is slow");
    expect(box.get(".thumb img").attributes("src")).toBe("askhuman-image://localhost//tmp/askhuman/gone/shot.png");
    expect(box.get(".rf-name").text()).toBe("report.txt");
    // Nothing in it can be edited or sent.
    expect(box.find("input, textarea, button.btn").exists()).toBe(false);
  });

  it("does not mark the question's options as chosen: the draft is not an answer", () => {
    const { wrapper } = mountDetail(cancelled());
    expect(wrapper.findAll(".q-block")[0].findAll(".options .option.selected")).toHaveLength(0);
  });

  it("opens a draft picture in the panel like a reply's, and a draft file in the system", async () => {
    const { wrapper, preview } = mountDetail(cancelled());
    await flushPromises();
    const thumb = wrapper.get(".draft-block .thumb-preview");
    expect(thumb.attributes("data-question-index")).toBe("0");
    expect(thumb.attributes("data-image-index")).toBe("0");
    click(thumb.element);
    await flushPromises();
    expect(preview.attach.isReplyPreview.value).toBe(true);
    expect(preview.attach.previewName.value).toBe("shot.png");
    expect(mocks.invoke).toHaveBeenCalledWith("popup_preview_layout", expect.objectContaining({ requestId: "history:gone", open: true }));
    await wrapper.get(".draft-block .reply-file").trigger("click");
    expect(openPath).toHaveBeenCalledExactlyOnceWith("/tmp/dir/report.txt");
  });

  it("lists the draft's notes apart from the ones that were sent, tagged as a draft", () => {
    const { wrapper } = mountDetail(cancelled());
    const section = wrapper.get("[data-draft-annotations]");
    expect(section.get(".annotations-label").text()).toBe("Annotations");
    expect(section.get(".draft-tag").text()).toBe("Draft");
    expect(section.get(".annotation-quote").text()).toBe("the plan");
    expect(section.get(".reply-text").text()).toBe("which plan?");
  });

  it("shows only what was typed: no notes, no section", () => {
    const { wrapper } = mountDetail(cancelled({ draft: draft({ annotations: [] }) }));
    expect(wrapper.find("[data-draft-annotations]").exists()).toBe(false);
    const bare = mountDetail(cancelled({ id: "bare", draft: draft({ annotations: undefined }) }));
    expect(bare.wrapper.find("[data-draft-annotations]").exists()).toBe(false);
  });

  it("copies the draft alone, question by question and then its notes", async () => {
    const { wrapper } = mountDetail(cancelled());
    await wrapper.get(".draft-line .copy-btn").trigger("click");
    await flushPromises();
    expect(copied).toEqual([[
      "Not submitted when the caller went away:",
      "",
      "Question 1/2",
      "Selected options:",
      "- No",
      "Reply:",
      "because it is slow",
      "Reply images:",
      "- /tmp/askhuman/gone/shot.png",
      "Reply files:",
      "- /tmp/dir/report.txt",
      "",
      "Annotations:",
      "> the plan",
      "which plan?",
    ].join("\n")]);
    expect(wrapper.get(".draft-line .copy-btn").text()).toBe("Copied");
  });

  it("puts the draft at the end of the whole-record copy", async () => {
    const { wrapper } = mountDetail(cancelled());
    await wrapper.get(".status-banner .copy-btn").trigger("click");
    await flushPromises();
    expect(copied).toHaveLength(1);
    const text = copied[0];
    expect(text.startsWith("Cancelled via Caller")).toBe(true);
    expect(text).toContain("Not answered");
    const heading = text.indexOf("Not submitted when the caller went away:");
    expect(heading).toBeGreaterThan(text.indexOf("Anything else?"));
    expect(text.slice(heading)).toContain("because it is slow");
    expect(text.slice(heading)).toContain("> the plan");
  });

  it("copies a single question's draft without numbering it", async () => {
    const single = cancelled({
      questions: [QUESTIONS[0]],
      draft: draft({ answers: [{ selectedOptions: [], userInput: "just this", images: [], files: [] }], annotations: [] }),
    });
    const { wrapper } = mountDetail(single);
    await wrapper.get(".draft-line .copy-btn").trigger("click");
    await flushPromises();
    expect(copied).toEqual(["Not submitted when the caller went away:\n\nReply:\njust this"]);
  });

  it("is absent from a cancelled entry that kept nothing, and from every sent one", () => {
    const nothing = mountDetail(cancelled({ id: "nothing", draft: undefined }));
    expect(nothing.wrapper.find(".draft-line").exists()).toBe(false);
    expect(nothing.wrapper.find(".draft-block").exists()).toBe(false);
    expect(nothing.wrapper.find("[data-draft-annotations]").exists()).toBe(false);
    // A draft never rides on an answer: a sent entry shows none even if the field were there.
    const sent = mountDetail(cancelled({
      id: "sent", action: "send", channel: "popup",
      answers: [{ selectedOptions: [], userInput: "the real answer", images: [], files: [] }, { selectedOptions: [], userInput: "", images: [], files: [] }],
    }));
    expect(sent.wrapper.find(".draft-line").exists()).toBe(false);
    expect(sent.wrapper.find(".draft-block").exists()).toBe(false);
    expect(sent.wrapper.get(".reply-text").text()).toBe("the real answer");
  });

  it("speaks the language of the interface", () => {
    i18n.global.locale.value = "zh";
    const { wrapper } = mountDetail(cancelled({ draft: draft({ claimedBy: "request-2" }) }));
    expect(wrapper.get(".draft-line").text()).toContain("调用方断开时未提交的内容");
    expect(wrapper.get(".draft-line-state").text()).toBe("已载入新提问");
    expect(wrapper.get(".draft-block .draft-tag").text()).toBe("草稿");
    expect(wrapper.get(".draft-line .copy-btn").text()).toBe("复制草稿");
  });
});
