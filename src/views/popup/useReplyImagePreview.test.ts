import { computed, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useReplyImagePreview } from "./useReplyImagePreview";

describe("reply image display lifecycle", () => {
  const probes: HTMLImageElement[] = [];
  beforeEach(() => {
    probes.length = 0;
    vi.stubGlobal("Image", function () {
      const probe = document.createElement("img");
      Object.defineProperties(probe, { naturalWidth: { value: 1200 }, naturalHeight: { value: 800 } });
      probes.push(probe); return probe;
    });
  });
  afterEach(() => vi.unstubAllGlobals());
  function setup() {
    const images = ref([[{ data: "first", mediaType: "image/png" }, { data: "second", mediaType: "image/png" }]]);
    const enabled = ref(true);
    return { enabled, state: useReplyImagePreview(images, computed(() => enabled.value), vi.fn()) };
  }
  it("discards stale decoding, restores each image's zoom and reuses its existing reply data", () => {
    const { state, enabled } = setup();
    state.select(0, 0); const late = probes[0].onload!;
    state.select(0, 1); late.call(probes[0], new Event("load"));
    expect(state.content.value).toBeNull();
    probes[1].onload!(new Event("load"));
    expect(state.content.value).toEqual({ kind: "image", url: "second", width: 1200, height: 800 });
    state.readingState.value!.image.zoom = 2;
    enabled.value = false;
    expect(state.content.value).toBeNull();
    enabled.value = true;
    expect(state.readingState.value!.image.zoom).toBe(2);
    expect(probes).toHaveLength(2);
    state.select(0, 0); probes[2].onload!(new Event("load"));
    expect(state.readingState.value!.image.zoom).toBe("fit");
    state.select(0, 1);
    expect(state.readingState.value!.image.zoom).toBe(2);
    state.dispose(); expect(state.content.value).toBeNull();
  });
  it("shows decode errors and cancels loading when disposed", () => {
    const { state } = setup(); state.select(0, 0);
    expect(state.loading.value).toBe(true);
    probes[0].onerror!(new Event("error"));
    expect(state.content.value).toEqual({ kind: "unavailable", reason: "imageFailed" });
    state.select(0, 1); const late = probes[1].onload!;
    state.dispose(); late.call(probes[1], new Event("load"));
    expect(state.loading.value).toBe(false); expect(state.content.value).toBeNull();
  });

  // The History window keeps paths and resolves them to display URLs on demand.
  describe("with a URL resolver", () => {
    const flush = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };
    function resolving(resolve: (image: { data: string }) => Promise<string>) {
      const images = ref([[{ data: "/tmp/a.png", mediaType: "" }, { data: "/tmp/b.png", mediaType: "" }]]);
      const enabled = ref(true);
      return { enabled, state: useReplyImagePreview(images, computed(() => enabled.value), vi.fn(), resolve) };
    }
    it("decodes and shows the resolved URL, once per image", async () => {
      const resolve = vi.fn(async (image: { data: string }) => `display://${image.data}`);
      const { state, enabled } = resolving(resolve);
      state.select(0, 0);
      expect(state.loading.value).toBe(true);
      expect(probes).toHaveLength(0);
      await flush();
      expect(probes[0].getAttribute("src")).toBe("display:///tmp/a.png");
      probes[0].onload!(new Event("load"));
      expect(state.loading.value).toBe(false);
      expect(state.content.value).toEqual({ kind: "image", url: "display:///tmp/a.png", width: 1200, height: 800 });
      enabled.value = false; enabled.value = true;
      expect(state.content.value).toEqual({ kind: "image", url: "display:///tmp/a.png", width: 1200, height: 800 });
      expect(resolve).toHaveBeenCalledTimes(1);
      expect(probes).toHaveLength(1);
    });
    it("reports an image whose URL cannot be resolved", async () => {
      const { state } = resolving(async () => { throw "stale"; });
      state.select(0, 0); await flush();
      expect(probes).toHaveLength(0);
      expect(state.loading.value).toBe(false);
      expect(state.content.value).toEqual({ kind: "unavailable", reason: "imageFailed" });
    });
    it("drops a URL that arrives after the image was left", async () => {
      const waiting: Array<(url: string) => void> = [];
      const { state } = resolving(() => new Promise<string>(done => { waiting.push(done); }));
      state.select(0, 0); state.select(0, 1);
      waiting[0]("display:///tmp/a.png"); await flush();
      expect(probes).toHaveLength(0);
      waiting[1]("display:///tmp/b.png"); await flush();
      expect(probes).toHaveLength(1);
      expect(probes[0].getAttribute("src")).toBe("display:///tmp/b.png");
      state.dispose();
    });
  });
});
