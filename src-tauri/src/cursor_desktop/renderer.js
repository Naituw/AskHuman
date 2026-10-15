(() => {
  "use strict";
  const key = "__ASKHUMAN_CURSOR_BACKGROUND_V1__";
  const owner = __ASKHUMAN_OWNER__;
  const revision = __ASKHUMAN_REVISION__;
  const lease = __ASKHUMAN_LEASE__;
  const verify = __ASKHUMAN_PREFLIGHT__;
  const existing = globalThis[key];
  if (existing) {
    if (existing.owner !== owner) throw new Error("Another AskHuman instance owns this window");
    if (existing.revision !== revision) throw new Error("A different adapter version requires restoration");
    if (existing.lease !== lease) throw new Error("A different adapter installation requires restoration");
    return existing.status();
  }
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
  const paired = dispatch.includes("_commitPairedCloudChildInboxCursor");
  const pairedContract = paired ? typeof prototype._commitPairedCloudChildInboxCursor === "function"
    : typeof prototype._commitPairedCloudChildInboxCursor === "undefined" &&
      /===\s*["']completed["']\)this\._backgroundWorkService\.ack\(/.test(dispatch);
  if (typeof descriptor?.value !== "function" || descriptor.configurable !== true ||
      !descriptor.value.toString().includes("_runWakeupAction") ||
      typeof prototype._runWakeupAction !== "function" ||
      typeof background.ack !== "function" || typeof background.nack !== "function" ||
      !dispatch.includes("_runBackgroundTaskCompletionAction") || !dispatch.includes('"completed"') ||
      !dispatch.includes("_backgroundWorkService.ack") || !dispatch.includes("_pendingAwaitTaskIdsByComposerId") ||
      !pairedContract) throw new Error("Unsupported background completion contract");

  let cancelled = false, installed = false, activation;
  let releasePreflight = () => {};
  const state = {
    owner, revision, lease, enabled: false, suppressed: 0, verification: undefined,
    surface: data._environmentService?.isGlass === true ? "agents" : "ide",
    intact: () => installed && Object.getOwnPropertyDescriptor(prototype, "_runBackgroundTaskCompletionAction")?.value === wrapper,
    status() {
      return { prepared: !cancelled, enabled: this.enabled, intact: this.intact(), surface: this.surface,
        suppressed: this.suppressed, verified: this.verification?.passed === true, checks: this.verification?.checks ?? 0 };
    },
    activate() {
      if (cancelled) return Promise.reject(new Error("Completion activation was cancelled"));
      if (installed) {
        if (!this.intact()) return Promise.reject(new Error("The completion handler changed after attachment"));
        return Promise.resolve(this.status());
      }
      if (activation) return activation;
      // Run only after the provider breakpoint has been resumed and detached.
      activation = (async () => {
        let timer;
        try {
          const verification = await Promise.race([
            verify({ prototype, background, handler: makeHandler({ enabled: true, suppressed: 0 }),
              id: "askhuman-preflight-" + lease, cancelled: () => cancelled,
              onCleanup: cleanup => { releasePreflight = cleanup; } }),
            new Promise((_, reject) => { timer = setTimeout(() => {
              cancelled = true; reject(new Error("Native completion preflight timed out"));
            }, 1500); }),
          ]);
          if (!verification?.passed || verification.checks < 6) throw new Error("Native completion preflight did not pass");
          if (cancelled || globalThis[key] !== state) throw new Error("Completion activation was cancelled");
          if (Object.getOwnPropertyDescriptor(prototype, "_runBackgroundTaskCompletionAction")?.value !== descriptor.value) {
            throw new Error("The completion handler changed during preflight");
          }
          state.verification = verification;
          Object.defineProperty(prototype, "_runBackgroundTaskCompletionAction", { ...descriptor, value: wrapper });
          installed = true; state.enabled = true;
          return state.status();
        } catch (error) {
          cancelled = true;
          if (globalThis[key] === state && !installed) delete globalThis[key];
          throw error;
        } finally { clearTimeout(timer); releasePreflight(); }
      })();
      return activation;
    },
    restore() {
      cancelled = true; this.enabled = false;
      releasePreflight();
      if (installed) {
        if (!this.intact()) return false;
        Object.defineProperty(prototype, "_runBackgroundTaskCompletionAction", descriptor);
        installed = false;
      }
      if (globalThis[key] === state) delete globalThis[key];
      return true;
    },
  };
  function makeHandler(control) {
    return async function (...args) {
      if (!control.enabled) return descriptor.value.apply(this, args);
      // Native dispatch still updates task outcomes, ACKs delivery, and settles any paired inbox.
      control.suppressed++;
      return "completed";
    };
  }
  const wrapper = makeHandler(state);
  globalThis[key] = state;
  return state.status();
})()
