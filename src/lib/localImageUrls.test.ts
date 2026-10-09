import { flushPromises } from "@vue/test-utils";
import { effectScope, ref, type EffectScope } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({
  localImageCreateScope: vi.fn(),
  localImagePrepare: vi.fn(),
  localImageReleaseAsset: vi.fn(async () => {}),
  localImageReleaseScope: vi.fn(async () => {}),
}));
vi.mock("./ipc", () => ipc);
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
}));
import { createLocalImageLoader, useLocalImageUrls } from "./localImageUrls";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
const prepared = (token: string) => ({ token, width: 10, height: 10 });

describe("local image display resources", () => {
  let scopes: EffectScope[];
  beforeEach(() => {
    scopes = [];
    vi.clearAllMocks();
    let sequence = 0;
    ipc.localImageCreateScope.mockReset().mockImplementation(async () => `scope-${++sequence}`);
    ipc.localImagePrepare.mockReset().mockImplementation(async (scope, path) => prepared(`${scope}:${path}`));
  });
  afterEach(async () => {
    scopes.forEach(scope => scope.stop());
    await flushPromises();
    vi.unstubAllGlobals();
    document.body.innerHTML = "";
  });
  function scoped<T>(make: () => T): T {
    const scope = effectScope();
    scopes.push(scope);
    return scope.run(make)!;
  }

  it("deduplicates paths and releases one asset without resetting the others", async () => {
    const loader = createLocalImageLoader();
    const first = loader.load("a.png");
    expect(loader.load("a.png")).toBe(first);
    await Promise.all([first, loader.load("b.png")]);
    loader.forget("a.png");
    await flushPromises();
    expect(ipc.localImagePrepare).toHaveBeenCalledTimes(2);
    expect(ipc.localImageReleaseAsset).toHaveBeenCalledWith("scope-1", "scope-1:a.png");
    expect(ipc.localImageReleaseScope).not.toHaveBeenCalled();
    expect(await loader.load("b.png")).toContain("scope-1:b.png");
    loader.dispose();
    await flushPromises();
    expect(ipc.localImageReleaseScope).toHaveBeenCalledWith("scope-1");
  });

  it("releases a scope created after cancellation without preparing the image", async () => {
    const pending = deferred<string>();
    ipc.localImageCreateScope.mockReturnValueOnce(pending.promise);
    const loader = createLocalImageLoader();
    const result = loader.load("a.png").catch(error => error);
    loader.dispose();
    pending.resolve("late-scope");
    expect(await result).toBe("stale");
    await flushPromises();
    expect(ipc.localImagePrepare).not.toHaveBeenCalled();
    expect(ipc.localImageReleaseScope).toHaveBeenCalledWith("late-scope");
  });

  it("releases a late token after removing and readding the same path", async () => {
    const pending = deferred<ReturnType<typeof prepared>>();
    ipc.localImagePrepare.mockReturnValueOnce(pending.promise).mockResolvedValueOnce(prepared("shared-token"));
    const loader = createLocalImageLoader();
    const old = loader.load("a.png").catch(error => error);
    await flushPromises();
    loader.forget("a.png");
    expect(await loader.load("a.png")).toContain("shared-token");
    pending.resolve(prepared("shared-token"));
    expect(await old).toBe("stale");
    await flushPromises();
    expect(ipc.localImageReleaseAsset).toHaveBeenCalledTimes(1);
    loader.forget("a.png");
    await flushPromises();
    expect(ipc.localImageReleaseAsset).toHaveBeenCalledTimes(2);
    loader.dispose();
  });

  it("retains unchanged previews when a selected file is removed and cleans up on unmount", async () => {
    const paths = ref(["a.png", "b.png"]);
    const state = scoped(() => useLocalImageUrls(paths));
    await flushPromises();
    paths.value = ["b.png"];
    await flushPromises();
    expect(state.urls.value).toEqual({ "b.png": "askhuman-image://localhost/scope-1:b.png" });
    expect(ipc.localImagePrepare).toHaveBeenCalledTimes(2);
    expect(ipc.localImageReleaseAsset).toHaveBeenCalledWith("scope-1", "scope-1:a.png");
    scopes[0].stop();
    await flushPromises();
    expect(ipc.localImageReleaseScope).toHaveBeenCalledWith("scope-1");
  });

  it("prevents an old record's late result from replacing the current record's image", async () => {
    const pending = deferred<ReturnType<typeof prepared>>();
    ipc.localImagePrepare.mockReturnValueOnce(pending.promise).mockResolvedValueOnce(prepared("current"));
    const owner = ref("first");
    const state = scoped(() => useLocalImageUrls(ref(["a.png"]), { owner }));
    await flushPromises();
    owner.value = "second";
    await flushPromises();
    expect(state.urls.value["a.png"]).toContain("current");
    pending.resolve(prepared("previous"));
    await flushPromises();
    expect(state.urls.value["a.png"]).toContain("current");
    expect(ipc.localImageReleaseScope).toHaveBeenCalledWith("scope-1");
    expect(ipc.localImageReleaseAsset).toHaveBeenCalledWith("scope-1", "previous");
  });

  it("waits for visible history targets and deduplicates multiple targets for one path", async () => {
    let notify!: IntersectionObserverCallback;
    const observe = vi.fn(), unobserve = vi.fn(), disconnect = vi.fn();
    vi.stubGlobal("IntersectionObserver", class {
      constructor(callback: IntersectionObserverCallback) { notify = callback; }
      observe = observe; unobserve = unobserve; disconnect = disconnect;
    });
    const state = scoped(() => useLocalImageUrls(ref(["a.png", "a.png", "b.png"]), { lazy: true }));
    const a = document.createElement("span"), b = document.createElement("span");
    document.body.append(a, b);
    state.observe(a, "a.png"); state.observe(b, "a.png");
    await flushPromises();
    expect(ipc.localImageCreateScope).not.toHaveBeenCalled();
    notify([{ target: a, isIntersecting: false }] as unknown as IntersectionObserverEntry[], {} as IntersectionObserver);
    await flushPromises();
    expect(ipc.localImagePrepare).not.toHaveBeenCalled();
    notify([a, b].map(target => ({ target, isIntersecting: true })) as unknown as IntersectionObserverEntry[], {} as IntersectionObserver);
    await flushPromises();
    expect(ipc.localImagePrepare).toHaveBeenCalledTimes(1);
    expect(state.urls.value["b.png"]).toBeUndefined();
    scopes[0].stop();
    expect(disconnect).toHaveBeenCalled();
  });

  it("ignores stale image errors and releases the current failed image", async () => {
    const paths = ref(["a.png"]);
    const state = scoped(() => useLocalImageUrls(paths));
    await flushPromises();
    const img = document.createElement("img");
    img.src = "askhuman-image://localhost/old-token";
    const event = new Event("error");
    Object.defineProperty(event, "target", { value: img });
    state.fail("a.png", event);
    expect(state.urls.value["a.png"]).toBeDefined();
    img.src = state.urls.value["a.png"];
    state.fail("a.png", event);
    await flushPromises();
    expect(state.failed.value["a.png"]).toBe(true);
    expect(state.urls.value["a.png"]).toBeUndefined();
    expect(ipc.localImageReleaseAsset).toHaveBeenCalledWith("scope-1", "scope-1:a.png");
    paths.value = ["a.png", "b.png"];
    await flushPromises();
    expect(ipc.localImagePrepare).toHaveBeenCalledTimes(2);
  });
});
