import { flushPromises, mount } from "@vue/test-utils";
import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import InteractSlot from "./InteractSlot.vue";
import { i18n } from "../../i18n";
import type { AgentRecord } from "../../lib/types";

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: vi.fn(async () => vi.fn()) }),
}));
const api = vi.hoisted(() => ({ message: vi.fn(), files: vi.fn() }));
vi.mock("../../lib/ipc", () => ({
  agentsMessage: api.message,
  codexDesktop: vi.fn(),
  codexDesktopAttachments: api.files,
}));
vi.mock("../interject/useInterjectAttachments", () => ({
  useInterjectAttachments: () => ({
    filePaths: ref([]),
    pastedImages: ref([]),
    composerImages: ref([]),
    composerFiles: ref([]),
    hasAttachments: ref(false),
    busy: ref(false),
    error: ref(""),
    reset: vi.fn(),
    chooseFiles: vi.fn(),
    onPaste: vi.fn(),
  }),
}));

function chat(extra: Partial<AgentRecord> = {}): AgentRecord {
  return {
    kind: "cursor",
    sessionId: "chat-1",
    startedAt: 1,
    lastActivity: 2,
    state: "ended",
    canMessage: true,
    ...extra,
  };
}

function slot(record: AgentRecord) {
  return mount(InteractSlot, {
    props: {
      record,
      submitBareEnter: false,
      pendingText: "",
      pendingCount: 0,
      pendingAttachmentCount: 0,
      newTaskSupported: false,
    },
    global: { plugins: [i18n], stubs: { ComposerAttachments: true } },
  });
}

const sendButton = (wrapper: ReturnType<typeof slot>) =>
  wrapper.findAll("button").find((b) => b.classes().includes("send"))!;
const textarea = (wrapper: ReturnType<typeof slot>) => wrapper.get("textarea").element as HTMLTextAreaElement;

beforeEach(() => {
  api.message.mockReset();
  api.files.mockReset().mockResolvedValue([]);
  i18n.global.locale.value = "en";
});

describe("messaging a Cursor chat that is not working", () => {
  it.each(["cursor", "pi", "codex"] as const)("sends to an ended %s session through its native channel", async (kind) => {
    const wrapper = slot(chat({ kind, ...(kind === "codex" ? { desktop: { connected: true } } : {}) }));
    expect(wrapper.get("textarea").attributes("placeholder")).toBe(i18n.global.t(kind === "codex" ? "desktop.message" : "console.messagePlaceholder"));
    await wrapper.get("textarea").setValue("  carry on  ");
    api.message.mockResolvedValueOnce({ status: "accepted" });
    await sendButton(wrapper).trigger("click");
    await flushPromises();
    expect(api.message).toHaveBeenCalledTimes(1);
    const [id, sentKind, sessionId, text, files] = api.message.mock.calls[0];
    expect(id).toMatch(/^[0-9a-f-]{36}$/);
    expect([sentKind, sessionId, text, files]).toEqual([kind, "chat-1", "carry on", []]);
    expect(wrapper.emitted("send")).toBeUndefined();
    expect(textarea(wrapper).value).toBe("");
    expect(wrapper.text()).toContain("Sent. The Agent has started working on it.");
  });

  it("works for an idle chat too", async () => {
    const wrapper = slot(chat({ state: "idle" }));
    await wrapper.get("textarea").setValue("next");
    api.message.mockResolvedValueOnce({ status: "accepted" });
    await sendButton(wrapper).trigger("click");
    await flushPromises();
    expect(api.message).toHaveBeenCalledTimes(1);
  });

  it("offers no composer unless the daemon says the chat can be messaged", () => {
    const ended = slot(chat({ canMessage: undefined }));
    expect(ended.find("textarea").exists()).toBe(false);
    expect(ended.text()).toContain(i18n.global.t("console.endedHint"));
    const idle = slot(chat({ state: "idle", canMessage: undefined }));
    expect(idle.find("textarea").exists()).toBe(false);
    expect(idle.text()).toContain(i18n.global.t("console.idleHint"));
  });

  it("still queues a message for a Cursor chat that is working", async () => {
    const wrapper = slot(chat({ state: "working", canMessage: undefined }));
    await wrapper.get("textarea").setValue("queued");
    await sendButton(wrapper).trigger("click");
    await flushPromises();
    expect(api.message).not.toHaveBeenCalled();
    expect(wrapper.emitted("send")).toHaveLength(1);
  });

  it("keeps the draft after a refusal and gives the next attempt a new id", async () => {
    const wrapper = slot(chat());
    await wrapper.get("textarea").setValue("hello");
    api.message.mockResolvedValueOnce({
      status: "rejected",
      reason: "chatNotLoaded",
      message: "Cursor has not opened this chat in any window, so nothing was sent.",
    });
    await sendButton(wrapper).trigger("click");
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toContain("nothing was sent");
    expect(textarea(wrapper).value).toBe("hello");
    expect(textarea(wrapper).readOnly).toBe(false);
    api.message.mockResolvedValueOnce({ status: "accepted" });
    await sendButton(wrapper).trigger("click");
    await flushPromises();
    expect(api.message).toHaveBeenCalledTimes(2);
    expect(api.message.mock.calls[1][0]).not.toBe(api.message.mock.calls[0][0]);
    expect(textarea(wrapper).value).toBe("");
  });

  it("retries the very same operation after an answer that never arrived", async () => {
    const wrapper = slot(chat());
    await wrapper.get("textarea").setValue("hello");
    api.message.mockRejectedValueOnce(new Error("response lost"));
    await sendButton(wrapper).trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("response lost");
    expect(textarea(wrapper).value).toBe("hello");
    api.message.mockResolvedValueOnce({ status: "accepted" });
    await sendButton(wrapper).trigger("click");
    await flushPromises();
    expect(api.message.mock.calls[1]).toEqual(api.message.mock.calls[0]);
    expect(api.files).toHaveBeenCalledTimes(1);
    expect(textarea(wrapper).value).toBe("");
  });

  it("will not send again until the user has checked a message that may have gone out", async () => {
    const wrapper = slot(chat());
    await wrapper.get("textarea").setValue("hello");
    api.message.mockResolvedValueOnce({
      status: "unknown",
      message: "The message may have been sent. Check the original chat in Cursor before sending another.",
    });
    await sendButton(wrapper).trigger("click");
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toContain("may have been sent");
    expect(textarea(wrapper).readOnly).toBe(true);
    expect(sendButton(wrapper).attributes("disabled")).toBeDefined();
    await sendButton(wrapper).trigger("click");
    expect(api.message).toHaveBeenCalledTimes(1);

    await wrapper.get("button.pending-revoke").trigger("click");
    expect(textarea(wrapper).readOnly).toBe(false);
    expect(textarea(wrapper).value).toBe("hello");
    api.message.mockResolvedValueOnce({ status: "accepted" });
    await sendButton(wrapper).trigger("click");
    await flushPromises();
    expect(api.message).toHaveBeenCalledTimes(2);
    expect(api.message.mock.calls[1][0]).not.toBe(api.message.mock.calls[0][0]);
  });
});
