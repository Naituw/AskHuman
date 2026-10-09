import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import type { AgentRecord } from "../../lib/types";
import InteractSlot from "./InteractSlot.vue";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
}));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: vi.fn(async () => vi.fn()) }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async (): Promise<string[] | null> => null) }));
vi.mock("../../lib/ipc", () => ({
  localImageCreateScope: vi.fn(async () => "console-scope"),
  localImagePrepare: vi.fn(async (_scope: string, path: string) => ({ token: path, width: 10, height: 10 })),
  localImageReleaseAsset: vi.fn(async () => {}),
  localImageReleaseScope: vi.fn(async () => {}),
}));
vi.mock("../../lib/theme", () => ({ fileToDataUrl: vi.fn(async () => "") }));

const record: AgentRecord = {
  kind: "codex",
  sessionId: "session-1",
  startedAt: 1,
  lastActivity: 2,
  state: "working",
};

afterEach(() => {
  document.body.innerHTML = "";
});

describe("InteractSlot composer", () => {
  it("uses the popup input interaction and a vector attachment action", async () => {
    const wrapper = mount(InteractSlot, {
      props: {
        record,
        submitBareEnter: false,
        pendingText: "",
        pendingCount: 0,
        pendingAttachmentCount: 0,
        newTaskSupported: false,
      },
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    await flushPromises();

    expect(
      wrapper.get(".answer-composer .input-wrap .textarea").element.tagName.toLowerCase(),
    ).toBe("textarea");
    expect(wrapper.get(".answer-composer .img-btn svg").element.tagName.toLowerCase()).toBe("svg");
    expect(wrapper.find(".attach-button").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("📎");
    wrapper.unmount();
  });

  it("handles image failures in the shared Interject composer without changing sent file paths", async () => {
    vi.mocked(openDialog).mockResolvedValueOnce(["/tmp/a.png"]);
    const wrapper = mount(InteractSlot, {
      props: { record, submitBareEnter: false, pendingText: "", pendingCount: 0, pendingAttachmentCount: 0, newTaskSupported: false },
      attachTo: document.body,
      global: { plugins: [i18n] },
    });
    await flushPromises();
    await wrapper.get(".img-btn").trigger("click");
    await flushPromises();
    expect(wrapper.get(".thumb img").attributes("src")).toBe("askhuman-image://localhost//tmp/a.png");
    await wrapper.get(".thumb img").trigger("error");
    await flushPromises();
    expect(wrapper.find(".thumb").exists()).toBe(false);
    expect(wrapper.get(".reply-files").text()).toContain("a.png");
    await wrapper.get(".send").trigger("click");
    await flushPromises();
    expect(wrapper.emitted("send")).toEqual([["", ["/tmp/a.png"], []]]);
    wrapper.unmount();
  });
});
