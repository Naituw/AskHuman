import { effectScope, nextTick, ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import { usePopupFind } from "./usePopupFind";
import type { AttachmentFindAdapter, AttachmentFindState } from "./attachmentFind";
import { isMac } from "../../lib/platform";

if (!globalThis.CSS) vi.stubGlobal("CSS", { escape: (value: string) => value });
const cleanups: (() => void)[] = [];
afterEach(() => { cleanups.splice(0).forEach(fn => fn()); document.body.replaceChildren(); });
function setup() {
  const content = document.createElement("div"); content.className = "content";
  content.innerHTML = '<p data-find-seg="message">alpha alpha</p>';
  const main = document.createElement("div"); main.className = "popup-main";
  main.appendChild(content); document.body.appendChild(main);
  const previewOpen = ref(true), previewIndex = ref<number | null>(0), active = ref(true);
  const scope = effectScope(); cleanups.push(() => scope.stop());
  const find = scope.run(() => usePopupFind({
    contentRef: ref(content), isConfirm: ref(false), confirmRequest: ref(null), messageText: ref("alpha alpha"),
    viewSource: ref(false), attachments: ref([]), questions: ref([]), whatsNext: ref(false), todoPrefix: ref(""),
    confirmChoiceTexts: ref([]), confirmTitle: ref(""), confirmSummary: ref(""), confirmToolName: ref(""),
    confirmBodyText: ref(""), currentQ: ref(0), verticalMode: ref(false), revealQuestion: vi.fn(),
    previewOpen, previewIndex, active,
  }))!;
  const adapter: AttachmentFindAdapter = {
    state: ref<AttachmentFindState>({ total: 3, current: 0, status: "ready" }),
    search: vi.fn(), go: vi.fn(), clear: vi.fn(), refresh: vi.fn(), selection: () => "", restoreFocus: vi.fn(),
  };
  find.registerAttachmentFind(adapter);
  return { content, find, adapter, previewOpen, previewIndex, active };
}
const key = (value: string, extra: KeyboardEventInit = {}) => new KeyboardEvent("keydown", {
  key: value, cancelable: true, [isMac ? "metaKey" : "ctrlKey"]: true, ...extra,
});
async function settle() { for (let i = 0; i < 5; i++) await nextTick(); }

describe("request-scoped find routing", () => {
  it.each(["Meta", "Control", "Alt", "Shift"])("does not let %s reset the region after opening an attachment from the question pane", modifier => {
    const { find, content, previewIndex } = setup();
    previewIndex.value = null;
    const file = document.createElement("button"); file.className = "attachment";
    content.appendChild(file); file.focus();
    file.addEventListener("pointerdown", find.noteFindInteraction);
    file.addEventListener("keydown", find.noteFindInteraction);
    file.dispatchEvent(new Event("pointerdown"));
    previewIndex.value = 0;
    file.dispatchEvent(new KeyboardEvent("keydown", { key: modifier, metaKey: modifier === "Meta",
      ctrlKey: modifier === "Control", altKey: modifier === "Alt", shiftKey: modifier === "Shift" }));
    const f = key("f"); file.dispatchEvent(f); find.handleFindKeydown(f);
    expect(find.findScope.value).toBe("attachment");
  });
  it("retains attachment reading through modifier keys at a stale answer focus, but actual answer typing restores question scope", () => {
    const { find, content } = setup();
    const editor = document.createElement("textarea"); content.appendChild(editor); editor.focus();
    editor.addEventListener("keydown", find.noteFindInteraction);
    find.noteFindRegion("attachment");
    editor.dispatchEvent(key("Meta", { metaKey: true, ctrlKey: false }));
    const f = key("f"); editor.dispatchEvent(f); find.handleFindKeydown(f);
    expect(find.findScope.value).toBe("attachment");
    editor.dispatchEvent(new KeyboardEvent("keydown", { key: "a" }));
    editor.dispatchEvent(f); find.handleFindKeydown(f);
    expect(find.findScope.value).toBe("question");
  });
  it("follows the last interacted region despite a stale editor focus, and keeps the query on scope changes", async () => {
    const { find, adapter, content } = setup();
    const editor = document.createElement("textarea"); document.body.appendChild(editor); editor.focus();
    find.noteFindRegion("attachment");
    find.handleFindKeydown(key("f")); find.onFindQueryInput("alpha");
    expect(find.findScope.value).toBe("attachment"); expect(find.findTotal.value).toBe(3);
    expect(adapter.search).toHaveBeenLastCalledWith("alpha", false, true);
    expect(content.querySelectorAll("mark")).toHaveLength(0);
    find.noteFindRegion("question"); find.handleFindKeydown(key("f")); await settle();
    expect(find.findQuery.value).toBe("alpha"); expect(find.findTotal.value).toBe(2);
    expect(content.querySelectorAll("mark")).toHaveLength(2);
    find.openFind(false, "attachment"); await settle();
    expect(content.querySelectorAll("mark")).toHaveLength(0);
  });
  it("keeps G on the explicitly selected scope and treats composing Enter as input", async () => {
    const { find, adapter } = setup(); find.openFind(false, "attachment");
    find.noteFindRegion("question");
    expect(find.handleFindKeydown(key("g", { shiftKey: true }))).toBe(true);
    expect(adapter.go).toHaveBeenCalledWith(-1);
    expect(find.handleFindKeydown(key("Enter", { isComposing: true }))).toBe(false);
    const bar = document.createElement("div"); bar.className = "popup-find-bar";
    const input = document.createElement("input"); bar.appendChild(input); document.body.appendChild(bar);
    const f = key("f"); input.dispatchEvent(f); find.handleFindKeydown(f);
    expect(find.findScope.value).toBe("attachment");
    await settle();
  });
  it("retains each request session while hidden, and closes only attachment find on collapsing preview", async () => {
    const { find, adapter, active, previewOpen } = setup();
    find.openFind(false, "attachment"); find.onFindQueryInput("alpha");
    active.value = false; await settle();
    expect(find.findActive.value).toBe(true); expect(find.findQuery.value).toBe("alpha");
    active.value = true; await settle();
    expect(adapter.search).toHaveBeenLastCalledWith("alpha", false, false, false);
    previewOpen.value = false; await settle();
    expect(find.findActive.value).toBe(false); expect(find.findQuery.value).toBe("");
    find.openFind(false, "question"); previewOpen.value = true; await settle();
    previewOpen.value = false; await settle(); expect(find.findActive.value).toBe(true);
  });
  it("does not advertise unsupported formats as zero matches and preserves native selection prefill", () => {
    const { find, adapter } = setup(); adapter.state.value = { total: 0, current: -1, status: "unsupported" };
    find.openFind(true, "attachment", "native text");
    expect(find.findQuery.value).toBe("native text"); expect(find.findNoMatch.value).toBe(false);
    expect(find.findCountLabel.value).toBe("");
    find.closeFind(); expect(find.findCaseSensitive.value).toBe(false);
  });
});
