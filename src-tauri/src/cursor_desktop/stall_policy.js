(() => {
  "use strict";
  const timeout = 300000;
  const retryWindow = 600000;
  const nativeRetryWindow = 180000;
  const turnOptionsType = "agent.v1.AgentHostTurnOptions";
  // A private unknown field of the local Renderer -> Agent Host turn-options message.
  // The ordinary timeout fields always carry the native fallback on that path.
  const field = 489175341;
  const fail = message => { throw new Error(message); };

  function retryRunner(native, protectedConfig) {
    if (typeof native !== "function" || !native.toString().includes("stallRetryElapsedMs:")) {
      fail("Unsupported native retry runner");
    }
    return function (options, ...rest) {
      const hooks = options?.hooks;
      if (protectedConfig.has(options?.runOptions?.stallDetectorConfig) && typeof hooks?.classify === "function") {
        const classify = hooks.classify;
        options = {
          ...options,
          hooks: {
            ...hooks,
            classify(error, context, ...args) {
              const elapsed = context?.stallRetryElapsedMs;
              if (Number.isFinite(elapsed)) {
                // Only the native stall branch reads this elapsed value.
                context = {
                  ...context,
                  stallRetryElapsedMs: elapsed >= retryWindow ? nativeRetryWindow : Math.min(elapsed, nativeRetryWindow - 1),
                };
              }
              return Reflect.apply(classify, this, [error, context, ...args]);
            },
          },
        };
      }
      return Reflect.apply(native, this, [options, ...rest]);
    };
  }

  function wire(message) {
    const type = message?.getType?.();
    if (type?.typeName !== turnOptionsType) return undefined;
    const binary = type.runtime?.bin;
    if (typeof binary?.listUnknownFields !== "function" ||
        typeof binary?.discardUnknownFields !== "function" ||
        typeof binary?.onUnknownField !== "function" ||
        type.fields?.list().some(item => item.no === field)) {
      fail("Unsupported Agent Host turn-options encoding");
    }
    return binary;
  }

  function marker(token) {
    if (!/^[0-9a-f]{32}$/.test(token)) fail("Invalid Agent Host attachment token");
    // The raw contents of a length-delimited protobuf field include the length prefix.
    return new Uint8Array([token.length, ...Array.from(token, char => char.charCodeAt(0))]);
  }

  function write(message, bytes) {
    const binary = wire(message);
    if (!binary || binary.listUnknownFields(message).some(item => item.no === field)) {
      fail("Unable to add the Agent Host timeout marker");
    }
    binary.onUnknownField(message, field, 2, bytes);
  }

  function take(message, expected) {
    const binary = wire(message);
    if (!binary) return false;
    const fields = binary.listUnknownFields(message);
    const found = fields.filter(item => item.no === field);
    if (found.length === 0) return false;
    binary.discardUnknownFields(message);
    for (const item of fields) {
      if (item.no !== field) binary.onUnknownField(message, item.no, item.wireType, item.data);
    }
    const tag = found[0];
    return !!expected && found.length === 1 && tag.wireType === 2 && tag.data.length === expected.length &&
      tag.data.every((byte, index) => byte === expected[index]);
  }

  const tagged = message => wire(message)?.listUnknownFields(message).some(item => item.no === field) === true;
  return { timeout, retryWindow, retryRunner, marker, write, take, tagged };
})()
