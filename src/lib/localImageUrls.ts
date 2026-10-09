import { computed, getCurrentScope, onScopeDispose, ref, watch, type Ref } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { localImageCreateScope, localImagePrepare, localImageReleaseAsset, localImageReleaseScope } from "./ipc";

interface ImageRead {
  promise: Promise<string>;
  scope: Promise<string>;
  token?: string;
}

/** Keep display resources independent of file paths and image bytes used for submission. */
export function createLocalImageLoader() {
  let generation = 0;
  let disposed = false;
  let scope: Promise<string> | null = null;
  const reads = new Map<string, ImageRead>();
  const release = (owner: Promise<string>, token: string) => {
    void owner.then(id => localImageReleaseAsset(id, token), () => {}).catch(() => {});
  };
  function load(path: string): Promise<string> {
    if (disposed) return Promise.reject("stale");
    const existing = reads.get(path);
    if (existing) return existing.promise;
    scope ??= localImageCreateScope();
    const owner = scope, epoch = generation;
    let record!: ImageRead;
    const current = () => !disposed && epoch === generation && reads.get(path) === record;
    const promise = owner.then(id => {
      if (!current()) throw "stale";
      return localImagePrepare(id, path);
    }).then(image => {
      if (!current()) { release(owner, image.token); throw "stale"; }
      record.token = image.token;
      return convertFileSrc(image.token, "askhuman-image");
    });
    record = { promise, scope: owner };
    reads.set(path, record);
    return promise;
  }
  function forget(path: string): void {
    const record = reads.get(path);
    reads.delete(path);
    if (record?.token) release(record.scope, record.token);
  }
  function reset(): void {
    generation++;
    reads.clear();
    if (scope) void scope.then(id => localImageReleaseScope(id), () => {}).catch(() => {});
    scope = null;
  }
  function dispose(): void { disposed = true; reset(); }
  return { load, forget, reset, dispose };
}

/** Selected files load immediately; long read-only history waits for visible thumbnail targets. */
export function useLocalImageUrls(paths: Readonly<Ref<readonly string[]>>, options: { owner?: Readonly<Ref<unknown>>; lazy?: boolean } = {}) {
  const urls = ref<Record<string, string>>({});
  const failed = ref<Record<string, boolean>>({});
  const loader = createLocalImageLoader();
  const jobs = new Map<string, Promise<string>>();
  const targets = new Map<Element, string>();
  let allowed = new Set<string>();
  let disposed = false;
  let observer: IntersectionObserver | null = null;

  function load(path: string): void {
    if (disposed || !allowed.has(path) || jobs.has(path) || failed.value[path]) return;
    const job = loader.load(path);
    jobs.set(path, job);
    void job.then(url => {
      if (!disposed && jobs.get(path) === job) urls.value[path] = url;
    }, () => {
      if (!disposed && jobs.get(path) === job) failed.value[path] = true;
    });
  }
  if (options.lazy && typeof IntersectionObserver !== "undefined") {
    observer = new IntersectionObserver(entries => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        observer?.unobserve(entry.target);
        const path = targets.get(entry.target);
        if (path) load(path);
      }
    }, { root: null, rootMargin: "240px 0px" });
  }
  const owner = computed(() => options.owner?.value);
  const stop = watch([paths, owner], ([values, key], previous) => {
    if (previous && key !== previous[1]) {
      loader.reset(); jobs.clear(); urls.value = {}; failed.value = {};
      observer?.disconnect();
    }
    const next = new Set(values);
    for (const path of allowed) {
      if (next.has(path)) continue;
      loader.forget(path); jobs.delete(path);
      delete urls.value[path]; delete failed.value[path];
    }
    allowed = next;
    if (!allowed.size) { loader.reset(); jobs.clear(); }
    for (const [target, path] of targets) {
      if (!allowed.has(path) || !target.isConnected) {
        observer?.unobserve(target); targets.delete(target);
      } else if (!jobs.has(path)) observer?.observe(target);
    }
    if (!observer) for (const path of allowed) load(path);
  }, { immediate: true });

  function observe(element: unknown, path: string): void {
    if (disposed || !(element instanceof Element) || !allowed.has(path)) return;
    if (targets.get(element) === path) return;
    targets.set(element, path);
    if (observer) { if (!jobs.has(path)) observer.observe(element); }
    else load(path);
  }
  function fail(path: string, event?: Event): void {
    if (!allowed.has(path)) return;
    if (event?.target instanceof HTMLImageElement && event.target.getAttribute("src") !== urls.value[path]) return;
    loader.forget(path);
    jobs.delete(path);
    delete urls.value[path];
    failed.value[path] = true;
  }
  function dispose(): void {
    disposed = true; stop(); observer?.disconnect(); targets.clear(); jobs.clear(); loader.dispose();
  }
  if (getCurrentScope()) onScopeDispose(dispose);
  return { urls, failed, observe, fail, dispose };
}
