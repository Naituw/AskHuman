import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { effectScope, type EffectScope } from "vue";

const mocks = vi.hoisted(() => ({
  readImageDataUrl: vi.fn(),
  localImageCreateScope: vi.fn(async () => "interject-scope"),
  localImagePrepare: vi.fn(async (_scope: string, path: string) => ({ token: path, width: 10, height: 10 })),
  localImageReleaseAsset: vi.fn(async () => {}),
  localImageReleaseScope: vi.fn(async () => {}),
}));

vi.mock("../../lib/ipc", () => mocks);
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { useInterjectAttachments } from "./useInterjectAttachments";

describe("useInterjectAttachments", () => {
  let scopes: EffectScope[];
  function createState() {
    const scope = effectScope();
    scopes.push(scope);
    return scope.run(() => useInterjectAttachments())!;
  }
  beforeEach(() => {
    scopes = [];
    vi.clearAllMocks();
    mocks.readImageDataUrl.mockReset();
    mocks.readImageDataUrl.mockResolvedValue("data:image/png;base64,aGVsbG8=");
  });
  afterEach(async () => {
    scopes.forEach(scope => scope.stop());
    await flushPromises();
  });

  it("prefills referenced files, previews images, and deduplicates paths", async () => {
    const state = createState();
    state.reset([
      {
        path: "/tmp/a.png",
        name: "a.png",
        size: 5,
        isImage: true,
        available: true,
      },
      {
        path: "/tmp/readme.md",
        name: "readme.md",
        size: 9,
        isImage: false,
        available: true,
      },
    ]);
    await flushPromises();

    expect(state.attachmentCount.value).toBe(2);
    expect(state.composerImages.value).toHaveLength(1);
    expect(state.composerImages.value[0].data).toBe("askhuman-image://localhost//tmp/a.png");
    expect(mocks.readImageDataUrl).not.toHaveBeenCalled();
    expect(state.composerFiles.value.map((file) => file.path)).toEqual(["/tmp/readme.md"]);

    state.appendPaths(["/tmp/readme.md", "/tmp/notes.txt"]);
    expect(state.filePaths.value).toEqual(["/tmp/a.png", "/tmp/readme.md", "/tmp/notes.txt"]);
  });

  it("keeps unavailable references visible and removable", () => {
    const state = createState();
    state.reset([
      {
        path: "/tmp/gone.pdf",
        name: "gone.pdf",
        size: 12,
        isImage: false,
        available: false,
      },
    ]);

    expect(state.composerFiles.value[0].available).toBe(false);
    state.removeComposerFile(0);
    expect(state.hasAttachments.value).toBe(false);
  });

  it("keeps the original path after a decode failure and releases reset resources", async () => {
    const state = createState();
    state.appendPaths(["/tmp/a.png"]);
    await flushPromises();
    state.onComposerImageError(0);
    await flushPromises();
    expect(state.composerImages.value).toHaveLength(0);
    expect(state.composerFiles.value[0].path).toBe("/tmp/a.png");
    expect(state.filePaths.value).toEqual(["/tmp/a.png"]);
    expect(state.pastedImages.value).toEqual([]);
    expect(mocks.localImageReleaseAsset).toHaveBeenCalledWith("interject-scope", "/tmp/a.png");
    state.reset();
    await flushPromises();
    expect(mocks.localImageReleaseScope).toHaveBeenCalledWith("interject-scope");
    expect(state.hasAttachments.value).toBe(false);
  });
});
