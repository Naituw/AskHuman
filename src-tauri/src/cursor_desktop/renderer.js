(() => {
  "use strict";
  const key = "__ASKHUMAN_CURSOR_BACKGROUND_V1__";
  const owner = __ASKHUMAN_OWNER__;
  const revision = __ASKHUMAN_REVISION__;
  const lease = __ASKHUMAN_LEASE__;
  const existing = globalThis[key];
  if (existing) {
    if (existing.owner !== owner) throw new Error("Another AskHuman instance owns this window");
    if (existing.revision !== revision) throw new Error("A different adapter version requires restoration");
    if (existing.lease !== lease) throw new Error("A different adapter installation requires restoration");
    if (!existing.intact()) throw new Error("The completion handler changed after attachment");
    existing.enabled = true;
    return existing.status();
  }

  // The provider closure belongs to ComposerDataService. Pause only our own call to obtain it.
  const data = this;
  if (!data?._instantiationService) throw new Error("Composer services are not ready");
  let agent, background;
  for (let scope = data._instantiationService; scope; scope = scope._parent) {
    for (const [id, service] of scope._services?._entries ?? []) {
      if (String(id) === "composerAgentService") agent ??= service;
      if (String(id) === "backgroundWorkService") background ??= service;
    }
  }
  if (!agent || !background) throw new Error("Background completion services are unavailable");
  let prototype = Object.getPrototypeOf(agent), descriptor;
  for (; prototype && prototype !== Object.prototype; prototype = Object.getPrototypeOf(prototype)) {
    descriptor = Object.getOwnPropertyDescriptor(prototype, "_runBackgroundTaskCompletionAction");
    if (descriptor) break;
  }
  const dispatch = prototype?._maybeDispatchBackgroundCompletions?.toString() ?? "";
  if (typeof descriptor?.value !== "function" || descriptor.configurable !== true ||
      !descriptor.value.toString().includes("_runWakeupAction") ||
      typeof prototype._runWakeupAction !== "function" ||
      typeof background.ack !== "function" || typeof background.nack !== "function" ||
      !dispatch.includes("_runBackgroundTaskCompletionAction") || !dispatch.includes('"completed"') ||
      !dispatch.includes("_backgroundWorkService.ack") || !dispatch.includes("_pendingAwaitTaskIdsByComposerId") ||
      !dispatch.includes("_commitPairedCloudChildInboxCursor")) {
    throw new Error("Unsupported background completion contract");
  }

  const state = {
    owner, revision, lease, enabled: true, suppressed: 0,
    surface: data._environmentService?.isGlass === true ? "agents" : "ide",
    intact: () => Object.getOwnPropertyDescriptor(prototype, "_runBackgroundTaskCompletionAction")?.value === wrapper,
    status() {
      return { enabled: this.enabled, intact: this.intact(), surface: this.surface, suppressed: this.suppressed };
    },
    restore() {
      this.enabled = false;
      if (!this.intact()) return false;
      Object.defineProperty(prototype, "_runBackgroundTaskCompletionAction", descriptor);
      delete globalThis[key];
      return true;
    },
  };
  async function wrapper(...args) {
    if (!state.enabled) return descriptor.value.apply(this, args);
    // The native dispatcher has already applied task state. "completed" lets it ACK the batch
    // and settle paired inbox bookkeeping without entering _runWakeupAction or creating a turn.
    state.suppressed++;
    return "completed";
  }
  Object.defineProperty(prototype, "_runBackgroundTaskCompletionAction", { ...descriptor, value: wrapper });
  globalThis[key] = state;
  return state.status();
})()
