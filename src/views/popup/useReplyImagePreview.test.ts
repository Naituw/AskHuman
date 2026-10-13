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
});
