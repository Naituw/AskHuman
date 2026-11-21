import { computed, ref } from "vue";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FileAttachment, LinkedFile } from "../../lib/types";

const mocks = vi.hoisted(() => ({
  openPath: vi.fn(async (_path: string) => {}),
  popupPreviewLink: vi.fn(async (_owner: string, _path: string): Promise<unknown> => ({ kind: "other" })),
  platform: { value: "mac" as "mac" | "windows" | "linux" },
}));
vi.mock("../../lib/ipc", () => ({ openPath: mocks.openPath, popupPreviewLink: mocks.popupPreviewLink }));
vi.mock("../../lib/platform", async importOriginal => {
  const actual = await importOriginal<typeof import("../../lib/platform")>();
  return { ...actual, primaryModifierPressed: (event: Pick<KeyboardEvent, "metaKey" | "ctrlKey">) => actual.primaryModifierPressed(event, mocks.platform.value) };
});
import { BACK_STACK_LIMIT, useLinkedFiles, type LinkActivation } from "./useLinkedFiles";

const attachment = (name: string): FileAttachment => ({ path: `/att/${name}`, name, size: 5, isImage: false });
const attachments = [attachment("one.md"), attachment("two.md")];
const click: LinkActivation = { detail: 1, metaKey: false, ctrlKey: false };

/** What the backend answers for `/p/<name>`: linked files are numbered after the two attachments, in the order asked. */
function backend() {
  const indexes = new Map<string, number>();
  mocks.popupPreviewLink.mockImplementation(async (_owner, path) => {
    if (path.endsWith("/dir")) return { kind: "other" };
    if (!indexes.has(path)) indexes.set(path, attachments.length + indexes.size);
    return { kind: "file", index: indexes.get(path)!, file: { path, name: path.split("/").pop()!, size: 1, isImage: false } } satisfies LinkedFile;
  });
}

function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
  const promise = new Promise<T>((a, b) => { resolve = a; reject = b; });
  return { promise, resolve, reject };
}

function setup() {
  const ownerId = ref("r1");
  const shown = ref<number | null>(null);
  const show = vi.fn((index: number) => { shown.value = index; });
  const collapse = vi.fn(() => { shown.value = null; });
  const showMenu = vi.fn();
  const linked = useLinkedFiles({
    ownerId: computed(() => ownerId.value), shown, attachmentAt: index => attachments[index] ?? null, show, collapse, showMenu,
  });
  return { linked, ownerId, shown, show, collapse, showMenu };
}

describe("links to local files", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.popupPreviewLink.mockReset();
    mocks.platform.value = "mac";
    backend();
  });
  afterEach(() => vi.restoreAllMocks());

  it("opens the file a link names in the panel, which then shows it as a file of its own", async () => {
    const { linked, show, shown } = setup();
    linked.activate("/p/a.md", click); await flushPromises();
    expect(mocks.popupPreviewLink).toHaveBeenCalledWith("r1", "/p/a.md");
    expect(show).toHaveBeenCalledWith(2);
    expect(linked.fileAt(2)).toMatchObject({ path: "/p/a.md", name: "a.md" });
    expect(linked.isLinkedPreview.value).toBe(true);
    expect(linked.activePaths.value).toEqual(new Set(["/p/a.md"]));
    expect(shown.value).toBe(2);
    linked.disposeLinked();
  });

  it("asks the backend every time for what it has not shown yet, and collapses at once when the link on show is clicked", async () => {
    const { linked, collapse, shown } = setup();
    linked.activate("/p/a.md", click); await flushPromises();
    linked.activate("/p/b.md", click); await flushPromises();
    expect(shown.value).toBe(3);
    // The link on show is selected, the other one is not.
    expect(linked.activePaths.value.has("/p/b.md")).toBe(true);
    expect(linked.activePaths.value.has("/p/a.md")).toBe(false);
    expect(mocks.popupPreviewLink).toHaveBeenCalledTimes(2);
    linked.activate("/p/b.md", click);
    expect(collapse).toHaveBeenCalledTimes(1);
    expect(shown.value).toBeNull();
    expect(linked.activePaths.value.size).toBe(0);
    expect(mocks.popupPreviewLink).toHaveBeenCalledTimes(2);
    // Opening it again goes back through the backend, which answers the index it already gave.
    linked.activate("/p/b.md", click); await flushPromises();
    expect(shown.value).toBe(3);
    expect(mocks.popupPreviewLink).toHaveBeenCalledTimes(3);
    linked.disposeLinked();
  });

  it("answers the first click of a double click only", async () => {
    const { linked, show } = setup();
    linked.activate("/p/a.md", { ...click, detail: 1 });
    linked.activate("/p/a.md", { ...click, detail: 2 });
    await flushPromises();
    expect(mocks.popupPreviewLink).toHaveBeenCalledTimes(1);
    expect(show).toHaveBeenCalledTimes(1);
    linked.disposeLinked();
  });

  it("opens the original in the system for ⌘ on macOS and Ctrl elsewhere, leaving the panel alone", async () => {
    const { linked, show, collapse } = setup();
    linked.activate("/p/a.md", { ...click, metaKey: true }); await flushPromises();
    expect(mocks.openPath).toHaveBeenCalledWith("/p/a.md");
    expect(mocks.popupPreviewLink).not.toHaveBeenCalled();
    // Ctrl-click is a right click on macOS: it is not the modifier there, so it is a plain click.
    linked.activate("/p/b.md", { ...click, ctrlKey: true }); await flushPromises();
    expect(mocks.openPath).toHaveBeenCalledTimes(1);
    expect(show).toHaveBeenLastCalledWith(2);
    mocks.platform.value = "windows";
    linked.activate("/p/c.md", { ...click, ctrlKey: true }); await flushPromises();
    expect(mocks.openPath).toHaveBeenLastCalledWith("/p/c.md");
    expect(mocks.popupPreviewLink).toHaveBeenCalledTimes(1);
    // ⌘ is not the modifier there: it is a plain click.
    linked.activate("/p/d.md", { ...click, metaKey: true }); await flushPromises();
    expect(mocks.openPath).toHaveBeenCalledTimes(2);
    expect(show).toHaveBeenLastCalledWith(3);
    expect(collapse).not.toHaveBeenCalled();
    linked.disposeLinked();
  });

  it("hands a directory to the system and has no menu for it", async () => {
    const { linked, show, showMenu } = setup();
    linked.activate("/p/dir", click); await flushPromises();
    expect(mocks.openPath).toHaveBeenCalledWith("/p/dir");
    expect(show).not.toHaveBeenCalled();
    linked.showLinkMenu("/p/dir"); await flushPromises();
    expect(showMenu).not.toHaveBeenCalled();
    linked.disposeLinked();
  });

  it("falls back to the system when registration fails or answers something unexpected", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const { linked, show } = setup();
    mocks.popupPreviewLink.mockRejectedValueOnce("too many linked files");
    linked.activate("/p/a.md", click); await flushPromises();
    expect(mocks.openPath).toHaveBeenLastCalledWith("/p/a.md");
    expect(error).toHaveBeenCalledTimes(1);
    for (const answer of [false, undefined, { kind: "file", index: -1, file: { path: "/x" } }, { kind: "file", index: 1.5, file: { path: "/x" } }, { kind: "file", index: 2 }]) {
      mocks.popupPreviewLink.mockResolvedValueOnce(answer);
      linked.activate("/p/b.md", click); await flushPromises();
    }
    expect(mocks.openPath).toHaveBeenCalledTimes(6);
    expect(show).not.toHaveBeenCalled();
    linked.disposeLinked();
  });

  it("lets the latest click win whatever order the answers come back in", async () => {
    const { linked, show } = setup();
    const first = deferred<unknown>(), second = deferred<unknown>();
    mocks.popupPreviewLink.mockReturnValueOnce(first.promise as Promise<LinkedFile>).mockReturnValueOnce(second.promise as Promise<LinkedFile>);
    linked.activate("/p/a.md", click);
    linked.activate("/p/b.md", click);
    second.resolve({ kind: "file", index: 3, file: { path: "/p/b.md", name: "b.md", size: 1, isImage: false } });
    await flushPromises();
    first.resolve({ kind: "file", index: 2, file: { path: "/p/a.md", name: "a.md", size: 1, isImage: false } });
    await flushPromises();
    expect(show).toHaveBeenCalledTimes(1);
    expect(show).toHaveBeenCalledWith(3);
    linked.disposeLinked();
  });

  it("does not let a jump that was overtaken by a collapse land after it", async () => {
    const { linked, show, collapse } = setup();
    linked.activate("/p/a.md", click); await flushPromises();
    const slow = deferred<unknown>();
    mocks.popupPreviewLink.mockReturnValueOnce(slow.promise as Promise<LinkedFile>);
    linked.activate("/p/b.md", click);
    linked.activate("/p/a.md", click); // the link on show: collapses, and is the latest click
    slow.resolve({ kind: "file", index: 3, file: { path: "/p/b.md", name: "b.md", size: 1, isImage: false } });
    await flushPromises();
    expect(collapse).toHaveBeenCalledTimes(1);
    expect(show).toHaveBeenCalledTimes(1);
    linked.disposeLinked();
  });

  it("forgets everything when the owner changes, and drops an answer that was still on its way", async () => {
    const { linked, ownerId, show, shown } = setup();
    linked.activate("/p/a.md", click); await flushPromises();
    expect(linked.activePaths.value.size).toBe(1);
    const late = deferred<unknown>();
    mocks.popupPreviewLink.mockReturnValueOnce(late.promise as Promise<LinkedFile>);
    linked.activate("/p/b.md", click);
    ownerId.value = "r2";
    expect(linked.fileAt(2)).toBeNull();
    late.resolve({ kind: "file", index: 3, file: { path: "/p/b.md", name: "b.md", size: 1, isImage: false } });
    await flushPromises();
    expect(show).toHaveBeenCalledTimes(1);
    shown.value = 2;
    expect(linked.activePaths.value.size).toBe(0);
    // The next click is asked of the new owner.
    linked.activate("/p/a.md", click); await flushPromises();
    expect(mocks.popupPreviewLink).toHaveBeenLastCalledWith("r2", "/p/a.md");
    linked.disposeLinked();
  });

  it("opens an attachment the text also links to as that attachment, and lights both", async () => {
    const { linked, show, shown } = setup();
    mocks.popupPreviewLink.mockResolvedValueOnce({ kind: "file", index: 1, file: { path: "/att/two.md", name: "two.md", size: 9, isImage: false } });
    linked.activate("/link/spelling/two.md", click); await flushPromises();
    expect(show).toHaveBeenCalledWith(1);
    // It is the attachment, with the attachment's own facts, and it is navigated like one.
    expect(linked.fileAt(1)).toBe(attachments[1]);
    expect(linked.isLinkedPreview.value).toBe(false);
    expect(linked.activePaths.value).toEqual(new Set(["/att/two.md", "/link/spelling/two.md"]));
    // Selecting the chip alone lights a link that spells the attachment's own path.
    shown.value = 0;
    expect(linked.activePaths.value).toEqual(new Set(["/att/one.md"]));
    linked.disposeLinked();
  });

  it("lights every link that names the file on show", async () => {
    const { linked } = setup();
    mocks.popupPreviewLink.mockResolvedValue({ kind: "file", index: 2, file: { path: "/real/a.rs", name: "a.rs", size: 1, isImage: false } });
    linked.activate("/p/a.rs", click); await flushPromises();
    linked.activate("/other/path/to/a.rs", { ...click, detail: 1 }); await flushPromises();
    // The second click named a file that is already on show: it collapsed the panel, which clears them.
    expect(linked.activePaths.value.size).toBe(0);
    linked.activate("/p/a.rs", click); await flushPromises();
    expect(linked.activePaths.value).toEqual(new Set(["/real/a.rs", "/p/a.rs", "/other/path/to/a.rs"]));
    linked.disposeLinked();
  });

  describe("inside the panel", () => {
    async function open(linked: ReturnType<typeof setup>["linked"], path: string, fromPanel: boolean) {
      linked.activate(path, click, fromPanel); await flushPromises();
    }

    it("goes Back through the jumps made in the panel, and only those", async () => {
      const { linked, shown, show } = setup();
      await open(linked, "/p/a.md", false);
      expect(linked.canGoBack.value).toBe(false);
      await open(linked, "/p/b.md", true);
      expect(shown.value).toBe(3);
      expect(linked.canGoBack.value).toBe(true);
      await open(linked, "/p/c.md", true);
      expect(shown.value).toBe(4);
      linked.goBack();
      expect(shown.value).toBe(3);
      expect(linked.canGoBack.value).toBe(true);
      linked.goBack();
      expect(shown.value).toBe(2);
      expect(linked.canGoBack.value).toBe(false);
      linked.goBack();
      expect(show).toHaveBeenCalledTimes(5);
      linked.disposeLinked();
    });

    it("changes nothing for a link to the file on show, and does not collapse for it", async () => {
      const { linked, collapse, show } = setup();
      await open(linked, "/p/a.md", false);
      await open(linked, "/p/a.md", true);
      await open(linked, "/p/a.md", true);
      expect(collapse).not.toHaveBeenCalled();
      expect(show).toHaveBeenCalledTimes(1);
      expect(linked.canGoBack.value).toBe(false);
      linked.disposeLinked();
    });

    it("starts over when anything else changes what the panel shows", async () => {
      const { linked, shown } = setup();
      await open(linked, "/p/a.md", false);
      await open(linked, "/p/b.md", true);
      expect(linked.canGoBack.value).toBe(true);
      // A chip, the menu's quick look or a link in the text: a new first file.
      shown.value = 0;
      expect(linked.canGoBack.value).toBe(false);
      await open(linked, "/p/c.md", false);
      await open(linked, "/p/d.md", true);
      expect(linked.canGoBack.value).toBe(true);
      await open(linked, "/p/e.md", false);
      expect(linked.canGoBack.value).toBe(false);
      await open(linked, "/p/f.md", true);
      expect(linked.canGoBack.value).toBe(true);
      // Closing the panel ends the trail too.
      shown.value = null;
      expect(linked.canGoBack.value).toBe(false);
      linked.disposeLinked();
    });

    it("keeps no trail for a jump whose panel was closed while it was on its way", async () => {
      const { linked, shown } = setup();
      await open(linked, "/p/a.md", false);
      const slow = deferred<unknown>();
      mocks.popupPreviewLink.mockReturnValueOnce(slow.promise as Promise<LinkedFile>);
      linked.activate("/p/b.md", click, true);
      shown.value = null;
      slow.resolve({ kind: "file", index: 3, file: { path: "/p/b.md", name: "b.md", size: 1, isImage: false } });
      await flushPromises();
      expect(shown.value).toBe(3);
      expect(linked.canGoBack.value).toBe(false);
      linked.disposeLinked();
    });

    it("walks back at most 50 files", async () => {
      const { linked, shown } = setup();
      await open(linked, "/p/0.md", false);
      for (let i = 1; i <= BACK_STACK_LIMIT + 5; i++) await open(linked, `/p/${i}.md`, true);
      let steps = 0;
      while (linked.canGoBack.value) { linked.goBack(); steps++; }
      expect(steps).toBe(BACK_STACK_LIMIT);
      // Files are numbered 2.. in the order asked: the oldest five steps fell off the trail.
      expect(shown.value).toBe(2 + 5);
      linked.disposeLinked();
    });

    it("lets Back overtake a jump that is still on its way", async () => {
      const { linked, shown } = setup();
      await open(linked, "/p/a.md", false);
      await open(linked, "/p/b.md", true);
      const slow = deferred<unknown>();
      mocks.popupPreviewLink.mockReturnValueOnce(slow.promise as Promise<LinkedFile>);
      linked.activate("/p/c.md", click, true);
      linked.goBack();
      slow.resolve({ kind: "file", index: 4, file: { path: "/p/c.md", name: "c.md", size: 1, isImage: false } });
      await flushPromises();
      expect(shown.value).toBe(2);
      linked.disposeLinked();
    });

    it("opens the original in the system for ⌘ or Ctrl, and does nothing for a double click", async () => {
      const { linked, shown } = setup();
      await open(linked, "/p/a.md", false);
      linked.activate("/p/b.md", { ...click, metaKey: true }, true);
      linked.activate("/p/c.md", { ...click, detail: 2 }, true);
      await flushPromises();
      expect(mocks.openPath).toHaveBeenCalledWith("/p/b.md");
      expect(mocks.popupPreviewLink).toHaveBeenCalledTimes(1);
      expect(shown.value).toBe(2);
      linked.disposeLinked();
    });
  });

  describe("right click", () => {
    it("shows the attachment menu for the file the link names", async () => {
      const { linked, showMenu, show } = setup();
      linked.showLinkMenu("/p/a.md"); await flushPromises();
      expect(showMenu).toHaveBeenCalledWith(2);
      // The menu does not open the panel by itself, but what it registered is remembered for the quick look.
      expect(show).not.toHaveBeenCalled();
      expect(linked.fileAt(2)).toMatchObject({ path: "/p/a.md" });
      linked.disposeLinked();
    });

    it("lets the latest menu win, and says nothing when registration fails", async () => {
      const error = vi.spyOn(console, "error").mockImplementation(() => {});
      const { linked, showMenu } = setup();
      const first = deferred<unknown>();
      mocks.popupPreviewLink.mockReturnValueOnce(first.promise as Promise<LinkedFile>);
      linked.showLinkMenu("/p/a.md");
      linked.showLinkMenu("/p/b.md"); await flushPromises();
      first.resolve({ kind: "file", index: 2, file: { path: "/p/a.md", name: "a.md", size: 1, isImage: false } });
      await flushPromises();
      expect(showMenu).toHaveBeenCalledTimes(1);
      expect(showMenu).toHaveBeenCalledWith(2);
      mocks.popupPreviewLink.mockRejectedValueOnce("nope");
      linked.showLinkMenu("/p/c.md"); await flushPromises();
      expect(showMenu).toHaveBeenCalledTimes(1);
      expect(error).toHaveBeenCalledTimes(1);
      expect(mocks.openPath).not.toHaveBeenCalled();
      linked.disposeLinked();
    });

    it("does not interrupt a click that is still on its way", async () => {
      const { linked, show, showMenu } = setup();
      const slow = deferred<unknown>();
      mocks.popupPreviewLink.mockReturnValueOnce(slow.promise as Promise<LinkedFile>);
      linked.activate("/p/a.md", click);
      linked.showLinkMenu("/p/b.md"); await flushPromises();
      slow.resolve({ kind: "file", index: 7, file: { path: "/p/a.md", name: "a.md", size: 1, isImage: false } });
      await flushPromises();
      expect(showMenu).toHaveBeenCalledTimes(1);
      expect(show).toHaveBeenCalledWith(7);
      linked.disposeLinked();
    });
  });

  it("answers nothing once disposed", async () => {
    const { linked, show } = setup();
    const slow = deferred<unknown>();
    mocks.popupPreviewLink.mockReturnValueOnce(slow.promise as Promise<LinkedFile>);
    linked.activate("/p/a.md", click);
    linked.disposeLinked();
    slow.resolve({ kind: "file", index: 2, file: { path: "/p/a.md", name: "a.md", size: 1, isImage: false } });
    await flushPromises();
    expect(show).not.toHaveBeenCalled();
    expect(mocks.openPath).not.toHaveBeenCalled();
  });
});
