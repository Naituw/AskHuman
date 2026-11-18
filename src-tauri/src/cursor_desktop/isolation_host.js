((owner, lease) => {
  "use strict";
  const key = "__ASKHUMAN_CURSOR_H2_ISOLATION__";
  const drainingKey = Symbol.for("askhuman.cursor.h2-isolation.draining");
  const prior = globalThis[key];
  if (prior) {
    if (prior.owner !== owner || prior.lease !== lease) throw new Error("Another bridge owns HTTP/2 isolation");
    return prior;
  }
  const path = "/agent.v1.AgentService/Run";
  const state = { owner, lease };
  let manager, descriptors, hooks, error;
  let activation, stopped = false;
  let protectedRuns = 0;
  if (globalThis[drainingKey]) throw new Error("Previous Cursor HTTP/2 isolation is draining");

  function policy(Manager, native) {
    const idleLimit = 4;
    const idleMs = 60_000;
    const pools = new WeakMap();
    const idle = new Set();
    const streams = new WeakMap();
    let enabled = true;
    let inFlight = 0;

    function retire(slot) {
      if (slot.retired) return;
      slot.retired = true;
      idle.delete(slot);
      clearTimeout(slot.timer);
      slot.pool.slots.delete(slot);
      Reflect.apply(native.abort, slot.manager, []);
    }

    function release(slot) {
      if (slot.retired) return;
      if (!enabled || slot.draining || idle.size >= idleLimit) {
        retire(slot);
      } else {
        idle.add(slot);
        slot.timer = setTimeout(() => retire(slot), idleMs);
        slot.timer.unref();
      }
    }

    function reserve(parent) {
      let pool = pools.get(parent);
      if (!pool) {
        pool = { slots: new Set() };
        pools.set(parent, pool);
      }
      for (const slot of pool.slots) {
        if (!idle.delete(slot)) continue;
        clearTimeout(slot.timer);
        return slot;
      }
      const manager = new Manager(parent.authority, parent.options, parent.http2SessionOptions);
      const slot = { manager, pool };
      pool.slots.add(slot);
      return slot;
    }

    const request = function (method, target, ...args) {
      if (!enabled || method !== "POST" || typeof target !== "string" || target.split("?", 1)[0] !== path) {
        return Reflect.apply(native.request, this, [method, target, ...args]);
      }
      return isolated.call(this, method, target, args);
    };
    async function isolated(method, target, args) {
      if (typeof this.authority !== "string" || !this.options || !("http2SessionOptions" in this)) {
        if (native.count) error = "Cursor HTTP/2 connection options changed";
        return Reflect.apply(native.request, this, [method, target, ...args]);
      }
      // Reserve before the native dial can yield. Neither the original pool nor a sibling Run
      // can select this manager while the request is pending or its stream is open.
      let slot;
      try { slot = reserve(this); }
      catch {
        if (native.count) error = "Cursor HTTP/2 connection could not be allocated";
        return Reflect.apply(native.request, this, [method, target, ...args]);
      }
      inFlight++;
      slot.busy = true;
      if (native.count) protectedRuns++;
      let stream;
      try {
        stream = await Reflect.apply(native.request, slot.manager, [method, target, ...args]);
      } catch (failure) {
        inFlight--;
        slot.busy = false;
        retire(slot);
        native.drained?.();
        throw failure;
      }
      streams.set(stream, slot.manager);
      const session = stream.session;
      const onGoaway = () => { slot.draining = true; };
      const close = () => {
        inFlight--;
        slot.busy = false;
        session?.off("goaway", onGoaway);
        release(slot);
        native.drained?.();
      };
      if (session?.closed || session?.destroyed) slot.draining = true;
      if (stream.closed || stream.destroyed) {
        close();
      } else {
        session?.once("goaway", onGoaway);
        stream.once("close", close);
      }
      return stream;
    }
    const notifyResponseByteRead = function (stream, ...args) {
      return Reflect.apply(native.notifyResponseByteRead, streams.get(stream) ?? this, [stream, ...args]);
    };
    const abort = function (...args) {
      for (const slot of pools.get(this)?.slots ?? []) retire(slot);
      return Reflect.apply(native.abort, this, args);
    };
    const state = function (...args) {
      const current = Reflect.apply(native.state, this, args);
      if (current === "open" || current === "connecting" || current === "verifying") return current;
      for (const slot of pools.get(this)?.slots ?? []) if (slot.busy) return "open";
      return current;
    };
    return {
      request, notifyResponseByteRead, abort, state, pending: () => inFlight,
      disable() {
        enabled = false;
        for (const slot of idle) retire(slot);
      },
    };
  }

  state.stage = value => {
    if (stopped) throw new Error("Cursor HTTP/2 isolation is draining");
    if (manager && value !== manager) throw new Error("Cursor HTTP/2 manager changed during attachment");
    if (manager) return true;
    if (typeof value !== "function") throw new Error("Cursor HTTP/2 manager is unavailable");
    const prototype = value.prototype;
    const source = Function.prototype.toString.call(value);
    if (!source.includes("http2SessionOptions") || !source.includes("registerRequest") ||
        !source.includes("pingIntervalMs")) throw new Error("Unsupported Cursor HTTP/2 manager");
    const candidates = Object.fromEntries(["request", "notifyResponseByteRead", "abort", "state"].map(name =>
      [name, Object.getOwnPropertyDescriptor(prototype, name)]));
    if (Object.values(candidates).some(entry => !entry?.writable || typeof entry.value !== "function")) {
      throw new Error("Cursor HTTP/2 manager methods are unavailable");
    }
    manager = value;
    descriptors = candidates;
    return true;
  };

  function intact() {
    return !!hooks && Object.keys(descriptors).every(name => manager.prototype[name] === hooks[name]);
  }

  state.status = () => ({
    pid: process.pid,
    state: error ? "unavailable" : intact() ? "active" : hooks ? "unavailable" : "pending",
    error: error ?? (hooks && !intact() ? "Cursor HTTP/2 hooks were replaced" : undefined),
    protectedRuns,
  });

  state.activate = () => activation ??= activate();
  async function activate() {
    if (hooks || error) return state.status();
    if (!manager) throw new Error("Cursor HTTP/2 manager was not discovered");
    const original = Object.fromEntries(Object.entries(descriptors).map(([name, entry]) => [name, entry.value]));
    const preflight = policy(manager, original);
    const http2 = process.getBuiltinModule("http2");
    const server = http2.createServer();
    const peers = new Set();
    let parent;
    try {
      server.on("session", peer => {
        peers.add(peer);
        peer.on("error", () => {});
      });
      server.on("stream", stream => {
        stream.on("error", () => {});
        stream.respond({ ":status": 200 });
      });
      await new Promise((resolve, reject) => {
        server.once("error", reject);
        server.listen(0, "127.0.0.1", resolve);
      });
      parent = new manager("http://127.0.0.1:" + server.address().port, {}, {});
      const verify = async () => {
        const [first, second] = await Promise.all([
          preflight.request.call(parent, "POST", path, {}, {}),
          preflight.request.call(parent, "POST", path, {}, {}),
        ]);
        first.on("error", () => {});
        second.on("error", () => {});
        if (!first.session || !second.session || first.session === second.session ||
            first.session.socket.localPort === second.session.socket.localPort) {
          throw new Error("Parallel Cursor Run streams shared a TCP connection");
        }
        const closed = new Promise(resolve => first.once("close", resolve));
        first.session.destroy();
        await closed;
        if (second.closed || second.destroyed) throw new Error("A sibling Cursor Run stream was closed");
      };
      let timer;
      try {
        await Promise.race([verify(), new Promise((_, reject) => {
          timer = setTimeout(() => reject(new Error("Cursor HTTP/2 preflight timed out")), 1500);
        })]);
      } finally {
        clearTimeout(timer);
      }
      if (stopped || globalThis[key] !== state || Object.keys(descriptors).some(name => manager.prototype[name] !== original[name])) {
        throw new Error("Cursor HTTP/2 manager changed during verification");
      }
      hooks = policy(manager, { ...original, count: true, drained: restoreDrained });
      for (const [name, entry] of Object.entries(descriptors)) {
        Object.defineProperty(manager.prototype, name, { ...entry, value: hooks[name] });
      }
      if (!intact()) throw new Error("Cursor HTTP/2 hooks could not be verified");
    } catch (failure) {
      restore();
      error = String(failure.message ?? failure).slice(0, 150);
    } finally {
      preflight.disable();
      if (parent) preflight.abort.call(parent);
      for (const peer of peers) peer.destroy();
      server.close();
    }
    return state.status();
  }

  function restore() {
    if (!hooks) return;
    hooks.disable();
    // Keep byte accounting for already isolated streams until they finish.
    if (manager.prototype.request === hooks.request) Object.defineProperty(manager.prototype, "request", descriptors.request);
    restoreDrained();
  }

  function restoreDrained() {
    if (manager.prototype.request === hooks.request || hooks.pending()) return;
    for (const name of ["notifyResponseByteRead", "abort", "state"]) {
      if (manager.prototype[name] === hooks[name]) Object.defineProperty(manager.prototype, name, descriptors[name]);
    }
    if (stopped && globalThis[drainingKey] === state) delete globalThis[drainingKey];
  }

  state.restore = () => {
    stopped = true;
    if (hooks?.pending()) globalThis[drainingKey] = state;
    restore();
    if (globalThis[key] === state) delete globalThis[key];
    return true;
  };
  globalThis[key] = state;
  return state;
})
