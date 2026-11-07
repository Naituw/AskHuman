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
  let runnerIntact, decoderIntact;
  const state = {
    owner, revision,
    attachRunner(get, set, classifier) {
      if (runnerIntact) {
        if (!runnerIntact()) throw new Error("The native Agent Host retry hook was replaced");
        return true;
      }
      const native = get();
      if (typeof classifier !== "function" || !classifier.toString().includes("stallRetryElapsedMs>=18e4")) {
        throw new Error("Unsupported native Agent Host retry classifier");
      }
      const wrapper = policy.retryRunner(native, configs);
      set(wrapper);
      if (get() !== wrapper) throw new Error("Could not attach the Agent Host retry hook");
      runnerIntact = () => get() === wrapper;
      return true;
    },
    attachDecoder(entry) {
      if (!runnerIntact?.()) throw new Error("The Agent Host retry hook is unavailable");
      if (!decoderIntact) {
        const descriptor = Object.getOwnPropertyDescriptor(entry, "read");
        if (descriptor?.writable !== true || typeof descriptor.value !== "function" ||
            !descriptor.value.toString().includes("stallFailTimeoutMs")) {
          throw new Error("Unsupported native Agent Host timeout decoder");
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
        Object.defineProperty(entry, "read", { ...descriptor, value: wrapper });
        decoderIntact = () => entry.read === wrapper;
      }
      if (!decoderIntact()) throw new Error("The native Agent Host timeout decoder was replaced");
      return { pid: process.pid, token };
    },
  };
  // The runtime is retained until the Host exits so a turn already in Cursor's queue can
  // read its snapshot after the switch is off. It has no effect on unmarked turns.
  globalThis[key] = state;
  return state;
})
