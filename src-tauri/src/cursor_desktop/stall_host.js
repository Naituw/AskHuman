((owner, revision) => {
  "use strict";
  const key = "__ASKHUMAN_CURSOR_STALL_HOST_V1__";
  const prior = globalThis[key];
  if (prior) {
    if (prior.owner !== owner || prior.revision !== revision) {
      throw new Error("A different AskHuman attachment owns the Agent Host stall guard");
    }
    return prior;
  }

  const policy = __ASKHUMAN_STALL_POLICY__;
  const token = process.getBuiltinModule("crypto").randomBytes(16).toString("hex");
  const marker = policy.marker(token);
  const configs = new WeakSet();
  const decoded = new WeakSet();
  let runnerIntact, decoderIntact, retryWindowMs, pendingRunner;
  const state = {
    owner, revision,
    status() {
      return runnerIntact?.() && decoderIntact?.() ? { pid: process.pid, token, retryWindowMs } : undefined;
    },
    attachRunner(get, set, classifier, nativeRetryWindow) {
      if (runnerIntact) {
        if (!runnerIntact()) throw new Error("The native Agent Host retry hook was replaced");
        return true;
      }
      const native = get();
      policy.verifyClassifier(classifier, nativeRetryWindow);
      retryWindowMs = Math.max(policy.retryWindow, nativeRetryWindow);
      const wrapper = policy.retryRunner(native, configs, nativeRetryWindow);
      // Keep both native bindings discoverable if the inspector disconnects between phases.
      pendingRunner = { get, set, native, wrapper };
      return true;
    },
    attachDecoder(entry) {
      if (!decoderIntact) {
        if (!pendingRunner || pendingRunner.get() !== pendingRunner.native) {
          throw new Error("The native Agent Host retry hook changed before attachment");
        }
        const descriptor = Object.getOwnPropertyDescriptor(entry, "read");
        if (descriptor?.writable !== true || typeof descriptor.value !== "function" ||
            !descriptor.value.toString().includes("stallFailTimeoutMs")) {
          throw new Error("Unsupported native Agent Host timeout decoder");
        }
        const probe = Reflect.apply(descriptor.value, entry, [{ stallAdvisoryTimeoutMs: 30000, stallFailTimeoutMs: 90000 }]);
        if (probe?.failTimeoutMs !== 90000 || probe.advisoryTimeoutMs !== 30000) {
          throw new Error("Native Agent Host timeout decoding preflight failed");
        }
        const wrapper = function (message, ...args) {
          // Drop the private field before Cursor can cache or forward its native message.
          let marked;
          try { marked = policy.take(message, marker); } catch { marked = false; }
          const native = Reflect.apply(descriptor.value, this, [message, ...args]);
          if (marked) decoded.add(message);
          if (!decoded.has(message) || !runnerIntact() || !Number.isSafeInteger(native?.failTimeoutMs) ||
              native.failTimeoutMs <= 0 || native.failTimeoutMs >= policy.timeout) return native;
          const extended = { ...native, failTimeoutMs: policy.timeout };
          configs.add(extended);
          return extended;
        };
        const runner = pendingRunner;
        try {
          runner.set(runner.wrapper);
          if (runner.get() !== runner.wrapper) throw new Error("Could not attach the Agent Host retry hook");
          runnerIntact = () => runner.get() === runner.wrapper;
          Object.defineProperty(entry, "read", { ...descriptor, value: wrapper });
          decoderIntact = () => entry.read === wrapper;
          if (!decoderIntact()) throw new Error("Could not attach the Agent Host timeout decoder");
          pendingRunner = undefined;
        } catch (error) {
          if (entry.read === wrapper) Object.defineProperty(entry, "read", descriptor);
          if (runner.get() === runner.wrapper) runner.set(runner.native);
          runnerIntact = decoderIntact = undefined;
          throw error;
        }
      }
      if (!runnerIntact?.()) throw new Error("The Agent Host retry hook is unavailable");
      if (!decoderIntact()) throw new Error("The native Agent Host timeout decoder was replaced");
      return state.status();
    },
  };
  // The runtime is retained until the Host exits so a turn already in Cursor's queue can
  // read its snapshot after the switch is off. It has no effect on unmarked turns.
  globalThis[key] = state;
  return state;
})
