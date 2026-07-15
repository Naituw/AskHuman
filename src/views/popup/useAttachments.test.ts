import { computed } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FileAttachment } from "../../lib/types";

const mocks = vi.hoisted(() => ({
  closePreview: vi.fn(() => Promise.resolve()),
  previewAttachments: vi.fn(() => Promise.resolve()),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("../../lib/ipc", () => ({
  closePreview: mocks.closePreview,
  fileIconDataUrl: vi.fn(() => Promise.resolve("")),
  openPath: vi.fn(() => Promise.resolve()),
  previewAttachments: mocks.previewAttachments,
  readImageDataUrl: vi.fn(() => Promise.resolve("")),
  showAttachmentMenu: vi.fn(() => Promise.resolve()),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, handler: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(name, handler);
    return Promise.resolve(() => mocks.listeners.delete(name));
  }),
}));

vi.mock("@crabnebula/tauri-plugin-drag", () => ({
  startDrag: vi.fn(() => Promise.resolve()),
}));

import { useAttachments } from "./useAttachments";

const attachment: FileAttachment = {
  path: "/tmp/report.pdf",
  name: "report.pdf",
  size: 128,
  isImage: false,
};

function key(key: string): KeyboardEvent {
  return new KeyboardEvent("keydown", { key, cancelable: true });
}

describe("useAttachments preview lifecycle", () => {
  beforeEach(() => {
    mocks.closePreview.mockClear();
    mocks.previewAttachments.mockClear();
    mocks.listeners.clear();
    document.body.replaceChildren();
  });

  it("toggles Quick Look with Space", () => {
    const attachments = useAttachments({ attachments: computed(() => [attachment]) });
    const item = document.createElement("button");
    document.body.appendChild(item);
    attachments.setAttRef(item, 0);
    attachments.selectedFile.value = 0;

    expect(attachments.handleAttachmentKey(key(" "))).toBe(true);
    expect(mocks.previewAttachments).toHaveBeenCalledWith([attachment.path], 0);
    expect(attachments.handleAttachmentKey(key(" "))).toBe(true);
    expect(mocks.closePreview).toHaveBeenCalledTimes(1);
  });

  it("keeps the preview and selection during popup interaction", () => {
    const attachments = useAttachments({ attachments: computed(() => [attachment]) });
    attachments.selectedFile.value = 0;
    attachments.handleAttachmentKey(key(" "));
    const target = document.createElement("textarea");

    attachments.onBackgroundClick({ target } as unknown as MouseEvent);

    expect(attachments.selectedFile.value).toBe(0);
    expect(mocks.closePreview).not.toHaveBeenCalled();
  });

  it("does not restore attachment focus after the user returns to the popup", async () => {
    const attachments = useAttachments({ attachments: computed(() => [attachment]) });
    const item = document.createElement("button");
    const focus = vi.spyOn(item, "focus");
    document.body.appendChild(item);
    attachments.setAttRef(item, 0);
    attachments.selectedFile.value = 0;
    attachments.handleAttachmentKey(key(" "));
    await attachments.initAttachmentPreviewListeners();
    focus.mockClear();

    attachments.onBackgroundClick({
      target: document.createElement("textarea"),
    } as unknown as MouseEvent);
    mocks.listeners.get("preview-closed")?.({ payload: undefined });
    await Promise.resolve();

    expect(focus).not.toHaveBeenCalled();
  });
});
