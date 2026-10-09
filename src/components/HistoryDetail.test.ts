import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import HistoryDetail from "./HistoryDetail.vue";
import { i18n } from "../i18n";
import type { HistoryEntry } from "../lib/types";
import { localImagePrepare, localImageReleaseScope, openPath, previewAttachments, readImageDataUrl } from "../lib/ipc";
import { startDrag } from "@crabnebula/tauri-plugin-drag";

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock("@crabnebula/tauri-plugin-drag", () => ({
  startDrag: vi.fn(async () => {}),
}));

vi.mock("../lib/ipc", () => ({
  closePreview: vi.fn(async () => {}),
  fileIconDataUrl: vi.fn(async () => ""),
  openPath: vi.fn(async () => {}),
  previewAttachments: vi.fn(async () => {}),
  readImageDataUrl: vi.fn(async () => ""),
  localImageCreateScope: vi.fn(async () => "history-scope"),
  localImagePrepare: vi.fn(async (_scope: string, path: string) => ({ token: path, width: 10, height: 10 })),
  localImageReleaseAsset: vi.fn(async () => {}),
  localImageReleaseScope: vi.fn(async () => {}),
  showAttachmentMenu: vi.fn(async () => {}),
}));

describe("HistoryDetail", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    i18n.global.locale.value = "en";
  });

  it("shows the prompt and predefined options for a cancelled request", () => {
    const entry: HistoryEntry = {
      id: "cancelled-request",
      timestampMs: 1_700_000_000_000,
      project: "",
      source: "",
      channel: "popup",
      action: "cancel",
      isMarkdown: false,
      message: { text: "", files: [] },
      questions: [
        {
          message: "Which release channel should we use?",
          predefinedOptions: [
            { text: "Stable", recommended: true },
            { text: "Beta", recommended: false },
          ],
        },
      ],
      answers: [],
    };

    const wrapper = mount(HistoryDetail, {
      props: { entry },
      global: { plugins: [i18n] },
    });

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
    const entry: HistoryEntry = {
      id: "mermaid-history",
      timestampMs: 1_700_000_000_000,
      project: "",
      source: "",
      channel: "popup",
      action: "send",
      isMarkdown: true,
      message: {
        text: "```mermaid\nsequenceDiagram\nA->>B: Hello\n```",
        files: [],
      },
      questions: [],
      answers: [],
    };

    const wrapper = mount(HistoryDetail, {
      props: { entry },
      global: {
        plugins: [i18n],
        stubs: {
          MarkdownContent: {
            props: ["source"],
            template: '<div class="markdown-stub">{{ source }}</div>',
          },
        },
      },
    });

    expect(wrapper.get(".markdown-stub").text()).toContain("sequenceDiagram");
    wrapper.unmount();
  });

  it("uses display URLs while preserving original-file preview, opening and drag icons", async () => {
    const entry: HistoryEntry = {
      id: "images", timestampMs: 1, project: "", source: "", channel: "popup", action: "send", isMarkdown: false,
      message: { text: "image", files: [{ path: "/tmp/a.png", name: "a.png", size: 10, isImage: true }] },
      questions: [], answers: [],
    };
    const wrapper = mount(HistoryDetail, { attachTo: document.body, props: { entry }, global: { plugins: [i18n] } });
    await flushPromises();
    expect(wrapper.get(".att-icon img").attributes("src")).toBe("askhuman-image://localhost//tmp/a.png");
    expect(readImageDataUrl).not.toHaveBeenCalled();
    expect(localImagePrepare).toHaveBeenCalledWith("history-scope", "/tmp/a.png");
    const attachment = wrapper.get(".attachment");
    await attachment.trigger("click");
    await attachment.trigger("keydown", { key: " " });
    expect(previewAttachments).toHaveBeenCalledWith(["/tmp/a.png"], 0);
    await attachment.trigger("dblclick");
    expect(openPath).toHaveBeenCalledWith("/tmp/a.png");
    await attachment.trigger("dragstart");
    const drag = vi.mocked(startDrag).mock.calls[0][0];
    expect(drag.item).toEqual(["/tmp/a.png"]);
    expect(drag.icon).toMatch(/^data:image\/png;base64,/);
    wrapper.unmount();
    await flushPromises();
    expect(localImageReleaseScope).toHaveBeenCalledWith("history-scope");
  });
});
