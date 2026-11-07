(() => {
  "use strict";
  const key = "__ASKHUMAN_CURSOR_STALL_V1__";
  const kernelKey = "__ASKHUMAN_CURSOR_STALL_RENDERER_KERNEL_V1__";
  const owner = __ASKHUMAN_OWNER__;
  const revision = __ASKHUMAN_REVISION__;
  const lease = __ASKHUMAN_LEASE__;
  const policy = __ASKHUMAN_STALL_POLICY__;
  const existing = globalThis[key];
  if (existing) {
    if (existing.owner !== owner || existing.revision !== revision || existing.lease !== lease) {
      throw new Error("A different AskHuman attachment owns the stall guard");
    }
    return existing.activate();
  }

  const glass = this._environmentService?.isGlass === true;
  const surface = glass ? "agents" : "ide";
  const state = { owner, revision, lease };
  let kernel, failure;
  const binding = glass ? {
    reader: F1e.prototype, host: eBl.prototype, classifier: hvk,
    runner: () => Cvk, setRunner: value => { Cvk = value; },
    encoder: () => XFl, setEncoder: value => { XFl = value; },
  } : {
    reader: Sme.prototype, host: fGo.prototype, classifier: E9_,
    runner: () => B9_, setRunner: value => { B9_ = value; },
    encoder: () => hGo, setEncoder: value => { hGo = value; },
  };
  const services = new Map();
  for (let scope = this._instantiationService; scope; scope = scope._parent) {
    for (const [id, value] of scope._services?._entries ?? []) {
      const name = String(id);
      if ((name === "extensionService" || name === "experimentService") && !services.has(name)) {
        services.set(name, value);
      }
    }
  }
  const extensions = services.get("extensionService");
  const experiments = services.get("experimentService");
  const environment = this._environmentService;
  const experimentKey = "nal_stall_detector_timeout_config";

  function manager() {
    const hosts = extensions._getExtensionHostManagers(1).filter(host =>
      host.containsExtension("anysphere.cursor-agent-host"));
    return hosts.length === 1 && Number.isSafeInteger(hosts[0].pid) && hosts[0].pid > 0 ? hosts[0] : undefined;
  }

  function makeKernel() {
    const readerName = "getStallDetectorTimeoutConfig";
    const warmName = "_noteTurnPrewarmTemplate";
    const reader = Object.getOwnPropertyDescriptor(binding.reader, readerName);
    const warm = Object.getOwnPropertyDescriptor(binding.host, warmName);
    const encoder = binding.encoder();
    const runner = binding.runner();
    if (reader?.writable !== true || !reader.value.toString().includes(experimentKey) ||
        warm?.writable !== true || !warm.value.toString().includes(".turnOptions.clone()") ||
        typeof encoder !== "function" || !encoder.toString().includes("stallFailTimeoutMs:") ||
        !binding.classifier.toString().includes("stallRetryElapsedMs>=") ||
        typeof extensions?._getExtensionHostManagers !== "function" ||
        typeof experiments?.getDynamicConfig !== "function") {
      throw new Error("Unsupported Cursor window stall contract");
    }
    const snapshots = new WeakMap();
    const wrappedRunner = policy.retryRunner(runner, snapshots);
    let target, failed;
    const wrappedReader = function (...args) {
      const native = Reflect.apply(reader.value, this, args);
      if (!target || this.environmentService.smokeTestNalStreamTimeoutMs !== undefined ||
          !Number.isSafeInteger(native?.failTimeoutMs) || native.failTimeoutMs <= 0 ||
          native.failTimeoutMs >= policy.timeout) return native;
      try {
        if (!intact() || manager() !== target.manager || target.manager.pid !== target.pid) return native;
        const result = { ...native, failTimeoutMs: policy.timeout };
        snapshots.set(result, { native, marker: target.marker });
        return result;
      } catch {
        return native;
      }
    };
    const wrappedEncoder = function (options, ...rest) {
      const snapshot = snapshots.get(options?.stallDetectorConfig);
      if (!snapshot) return Reflect.apply(encoder, this, [options, ...rest]);
      const message = Reflect.apply(encoder, this, [{ ...options, stallDetectorConfig: snapshot.native }, ...rest]);
      try { policy.write(message, snapshot.marker); }
      catch {
        target = undefined;
        failed = "Cursor could not encode the local stall policy";
      }
      return message;
    };
    const wrappedWarm = function (session, request, ...rest) {
      // Cursor caches these templates for future turns. Strip the turn's private marker from the
      // copy while leaving the primary send and any already-snapshotted follow-up untouched.
      const options = request?.turnOptions;
      if (policy.tagged(options)) {
        const copy = options.clone();
        policy.take(copy);
        request = { ...request, turnOptions: copy };
      }
      return Reflect.apply(warm.value, this, [session, request, ...rest]);
    };
    const intact = () => binding.runner() === wrappedRunner && binding.encoder() === wrappedEncoder &&
      binding.reader[readerName] === wrappedReader && binding.host[warmName] === wrappedWarm;
    try {
      binding.setRunner(wrappedRunner);
      binding.setEncoder(wrappedEncoder);
      Object.defineProperty(binding.reader, readerName, { ...reader, value: wrappedReader });
      Object.defineProperty(binding.host, warmName, { ...warm, value: wrappedWarm });
      if (!intact()) throw new Error("Cursor window stall hooks could not be verified");
    } catch (error) {
      if (binding.runner() === wrappedRunner) binding.setRunner(runner);
      if (binding.encoder() === wrappedEncoder) binding.setEncoder(encoder);
      if (binding.reader[readerName] === wrappedReader) Object.defineProperty(binding.reader, readerName, reader);
      if (binding.host[warmName] === wrappedWarm) Object.defineProperty(binding.host, warmName, warm);
      throw error;
    }
    return {
      owner, revision, intact,
      failure: () => failed,
      disable() { target = undefined; },
      ready(host) { return host && target?.manager === host && target.pid === host.pid; },
      set(host, token) { target = { manager: host, pid: host.pid, marker: policy.marker(token) }; },
    };
  }

  try {
    kernel = globalThis[kernelKey];
    if (!kernel) {
      kernel = makeKernel();
      globalThis[kernelKey] = kernel;
    }
    if (kernel.owner !== owner || kernel.revision !== revision || !kernel.intact()) {
      throw new Error("Cursor window stall hooks changed after attachment");
    }
  } catch (error) {
    failure = String(error.message).slice(0, 160);
  }
  state.activate = () => {
    const base = { prepared: true, surface };
    if (failure || !kernel?.intact() || kernel.failure()) {
      return { ...base, state: "unavailable", error: failure || kernel?.failure() || "Cursor window stall hooks were replaced" };
    }
    const raw = experiments.getDynamicConfig(experimentKey, { disableExposureLog: true });
    const native = environment.smokeTestNalStreamTimeoutMs ?? raw?.failTimeoutMs;
    if (!Number.isSafeInteger(native)) return { ...base, state: "unavailable", error: "The native stall timeout is unavailable" };
    if (environment.smokeTestNalStreamTimeoutMs !== undefined || native <= 0 || native >= policy.timeout) {
      return { ...base, state: "notApplicable", timeoutMs: native };
    }
    const host = manager();
    if (!kernel.ready(host)) {
      kernel.disable();
      return { ...base, state: "pending", hostPid: host?.pid };
    }
    return { ...base, state: "active", timeoutMs: policy.timeout, retryWindowMs: policy.retryWindow };
  };
  state.setHost = (pid, token) => {
    const host = manager();
    if (failure || !kernel?.intact() || host?.pid !== pid) return state.activate();
    kernel.set(host, token);
    return state.activate();
  };
  state.restore = () => {
    if (kernel?.owner === owner && kernel.revision === revision) kernel.disable();
    if (globalThis[key] === state) delete globalThis[key];
    return true;
  };
  globalThis[key] = state;
  return state.activate();
})()
