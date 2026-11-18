(async () => {
  "use strict";
  const config = __ASKHUMAN_CONFIG__;
  const rendererSource = __ASKHUMAN_RENDERER__;
  const key = `__ASKHUMAN_CURSOR_BRIDGE_${config.owner}__`;
  const rendererKey = "__ASKHUMAN_CURSOR_BACKGROUND_V1__";
  const previous = globalThis[key];
  if (previous) {
    if (previous.protocol !== 1 || typeof previous.restore !== "function" || previous.status?.().pid !== process.pid) {
      throw new Error("Existing bridge identity differs");
    }
    if (previous.token === config.token && previous.revision === config.revision && previous.status().enabled) return previous.status();
    // Verified main-process bootstrap can recover this home's own bridge after losing its file.
    if (!(await previous.restore()).restored) throw new Error("Previous adapter still needs restoration");
    for (let attempt = 0; attempt < 20 && process.getBuiltinModule("fs").existsSync(config.socket); attempt++) {
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    if (process.getBuiltinModule("fs").existsSync(config.socket)) throw new Error("Previous bridge is still closing");
  }
  const require = process.getBuiltinModule("module").createRequire(config.appPackage);
  const electron = require("electron");
  const net = process.getBuiltinModule("net");
  const fs = process.getBuiltinModule("fs");
  const crypto = process.getBuiltinModule("crypto");
  const lease = crypto.randomUUID();
  const installSource = rendererSource.replace("__ASKHUMAN_LEASE__", JSON.stringify(lease));
  const windows = new Map();
  // The completion adapter is the primary guard; its window fields keep their meaning. Optional
  // extra guards share this bridge's attachment and lifecycle but never each other's failures.
  const primary = {
    key: rendererKey, source: installSource, label: "Completion adapter",
    ready: state => state?.enabled && state.intact && state.verified,
    notReady: "Completion adapter did not pass native preflight",
  };
  const guards = (Array.isArray(config.guards) ? config.guards : []).map(guard => ({
    name: String(guard.name), key: String(guard.key), label: `${guard.name} guard`,
    source: String(guard.source).replace("__ASKHUMAN_LEASE__", JSON.stringify(lease)),
    // Unavailable is a settled outcome (a contract that does not match is not retried).
    ready: state => ["active", "notApplicable", "unavailable"].includes(state?.state) ||
      (guard.name === "stall" && state?.state === "pending"),
    notReady: `The ${guard.name} guard did not settle`,
  }));
  let enabled = true, stopped = false, refreshPromise, registration;
  let attachStallHost;
  const isolationHosts = new Map();
  const bounded = (promise, ms, label) => {
    let timer;
    return Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(label)), ms);
    })]).finally(() => clearTimeout(timer));
  };
  const evaluate = (wc, expression) => bounded(wc.executeJavaScript(expression), 3000, "Window evaluation timed out");
  const readState = (wc, key = rendererKey) => evaluate(wc, `(async () => {
    const s = globalThis[${JSON.stringify(key)}];
    if (!s) return null;
    if (s.owner !== ${JSON.stringify(config.owner)}) throw new Error("Another AskHuman instance owns this window");
    if (s.revision !== ${JSON.stringify(config.revision)}) throw new Error("A different adapter version requires restoration");
    if (s.lease !== ${JSON.stringify(lease)}) throw new Error("A different adapter installation requires restoration");
    return await s.activate();
  })()`);
  const restoreState = (wc, key) => evaluate(wc, `(() => {
    const s = globalThis[${JSON.stringify(key)}];
    if (!s) return true;
    if (s.owner !== ${JSON.stringify(config.owner)} || s.revision !== ${JSON.stringify(config.revision)} || s.lease !== ${JSON.stringify(lease)}) return false;
    return s.restore();
  })()`);
  // Every guard is restored; one that cannot be restored keeps the bridge available for a retry.
  const restoreWindow = async wc => (await Promise.all(
    [rendererKey, ...guards.map(guard => guard.key)].map(key => restoreState(wc, key)),
  )).every(Boolean);

  async function unwrapStallKernelV1(command) {
    const scopes = async objectId => {
      const props = await command("Runtime.getProperties", { objectId, ownProperties: true });
      const listId = props.internalProperties?.find(item => item.name === "[[Scopes]]")?.value?.objectId;
      if (!listId) return [];
      const list = await command("Runtime.getProperties", { objectId: listId, ownProperties: true });
      const out = [];
      for (const item of list.result ?? []) {
        if (item.value?.objectId && !/^Global\b/.test(item.value.description ?? "")) {
          const sp = await command("Runtime.getProperties", { objectId: item.value.objectId, ownProperties: true });
          out.push(Object.fromEntries((sp.result ?? []).map(p => [p.name, p.value])));
        }
      }
      return out;
    };
    const intactId = (await command("Runtime.evaluate", {
      expression: "globalThis.__ASKHUMAN_CURSOR_STALL_RENDERER_KERNEL_V1__?.intact", returnByValue: false,
    }))?.result?.objectId;
    if (!intactId) throw new Error("Previous Cursor window stall kernel is unavailable");
    const intactScopes = await scopes(intactId);
    const kernelScope = intactScopes.find(s => s.reader && s.warm && s.encoder && s.wrappedRunner);
    const outerScope = intactScopes.find(s => s.binding);
    const runnerScope = kernelScope ? (await scopes(kernelScope.wrappedRunner.objectId)).find(s => s.native) : undefined;
    if (!kernelScope || !outerScope || !runnerScope) {
      throw new Error("Previous Cursor window stall kernel could not be unwrapped");
    }
    await command("Runtime.callFunctionOn", {
      objectId: outerScope.binding.objectId,
      functionDeclaration: `function(runner, encoder, reader, warm, snapshots) {
        this.setRunner(runner);
        this.setEncoder(encoder);
        Object.defineProperty(this.reader, "getStallDetectorTimeoutConfig", reader);
        Object.defineProperty(this.host, "_noteTurnPrewarmTemplate", warm);
        globalThis.__ASKHUMAN_CURSOR_STALL_RENDERER_SNAPSHOTS_V1__ = snapshots;
        delete globalThis.__ASKHUMAN_CURSOR_STALL_RENDERER_KERNEL_V1__;
        delete globalThis.__ASKHUMAN_CURSOR_STALL_V1__;
      }`,
      arguments: [
        { objectId: runnerScope.native.objectId },
        { objectId: kernelScope.encoder.objectId },
        { objectId: kernelScope.reader.objectId },
        { objectId: kernelScope.warm.objectId },
        ...(kernelScope.snapshots?.objectId ? [{ objectId: kernelScope.snapshots.objectId }] : []),
      ],
      returnByValue: true,
    });
  }

  async function install(wc, guard = primary) {
    if (wc.debugger.isAttached() || wc.isDevToolsOpened()) throw new Error("Window debugger is in use");
    let attached = false, breakpoint, listener, oursPaused = false;
    const group = `askhuman-${crypto.randomUUID()}`;
    const command = (method, params = {}) => bounded(wc.debugger.sendCommand(method, params), 3000, "Debugger command timed out");
    try {
      wc.debugger.attach("1.3");
      attached = true;
      await command("Debugger.enable");
      const fn = (await command("Runtime.evaluate", {
        expression: "globalThis.__SSG_COMPOSER_PROVIDER__?.getStates", objectGroup: group,
      })).result;
      if (fn?.type !== "function" || !fn.objectId) throw new Error("Composer provider is not ready");
      breakpoint = (await command("Debugger.setBreakpointOnFunctionCall", { objectId: fn.objectId })).breakpointId;
      const paused = new Promise((resolve, reject) => {
        listener = async (_event, method, params) => {
          if (method !== "Debugger.paused") return;
          oursPaused = true;
          try {
            if (!params.hitBreakpoints?.includes(breakpoint)) throw new Error("Unexpected debugger pause");
            const frame = params.callFrames?.[0];
            if (!frame) throw new Error("Composer provider frame is unavailable");
            let source = guard.source;
            if (guard.name === "stall") {
              const prior = await command("Debugger.evaluateOnCallFrame", {
                callFrameId: frame.callFrameId,
                expression: `(() => {
                  const k = globalThis.__ASKHUMAN_CURSOR_STALL_RENDERER_KERNEL_V1__;
                  if (!k) return "none";
                  if (k.owner !== ${JSON.stringify(config.owner)}) return "foreign";
                  if (k.contract === 2) return "ready";
                  if (typeof k.unwrap === "function") return k.unwrap() ? "none" : "failed";
                  return "v1";
                })()`,
                returnByValue: true,
              });
              if (prior.result?.value === "v1") await unwrapStallKernelV1(command);
              const binding = prior.result?.value === "ready" ? "undefined" : await eval(config.stallDiscovery)(command).renderer(frame);
              source = source.replace("__ASKHUMAN_STALL_BINDING__", () => binding);
            }
            const result = await command("Debugger.evaluateOnCallFrame", {
              callFrameId: frame.callFrameId, expression: source, returnByValue: true,
            });
            if (result.exceptionDetails) {
              const detail = result.exceptionDetails.exception?.description?.split("\n")[0];
              throw new Error(detail?.slice(0, 180) || `${guard.label} contract check failed`);
            }
            resolve(result.result?.value);
          } catch (error) { reject(error); }
          finally {
            try { await command("Debugger.removeBreakpoint", { breakpointId: breakpoint }); breakpoint = undefined; }
            catch { /* The window may have reloaded. */ }
            try { await command("Debugger.resume"); oursPaused = false; }
            catch { /* A destroyed context has already resumed. */ }
          }
        };
        wc.debugger.on("message", listener);
      });
      // Observe both promises immediately so teardown cannot leave an unhandled rejection.
      const invoke = command("Runtime.evaluate", {
        expression: "globalThis.__SSG_COMPOSER_PROVIDER__.getStates()", returnByValue: true,
      });
      const [state] = await bounded(Promise.all([paused, invoke]), 5000, "Composer discovery timed out");
      if (!state?.prepared) throw new Error(`${guard.label} was not prepared`);
    } finally {
      if (listener) wc.debugger.removeListener("message", listener);
      if (breakpoint) try { await command("Debugger.removeBreakpoint", { breakpointId: breakpoint }); } catch {}
      if (oursPaused) try { await command("Debugger.resume"); } catch {}
      if (attached) {
        try { await command("Runtime.releaseObjectGroup", { objectGroup: group }); } catch {}
        try { await command("Debugger.disable"); } catch {}
        try { wc.debugger.detach(); } catch {}
      }
    }
    const state = await readState(wc, guard.key);
    if (!guard.ready(state)) throw new Error(guard.notReady);
    return state;
  }

  function track(wc) {
    if (wc.isDestroyed() || wc.getType() !== "window" || windows.has(wc.id)) return;
    const record = { wc, phase: "attaching", generation: 0, suppressed: 0, retryAt: 0, attempts: 0, guards: {} };
    record.reload = () => {
      record.generation++;
      record.phase = "attaching";
      record.retryAt = 0;
      record.attempts = 0;
      record.guards = {};
      delete record.error;
      delete record.isolation;
      delete record.isolationWaitSince;
    };
    record.destroy = () => windows.delete(wc.id);
    wc.on("did-start-loading", record.reload);
    wc.on("destroyed", record.destroy);
    windows.set(wc.id, record);
  }

  function refresh() {
    if (stopped || !enabled) return Promise.resolve();
    if (refreshPromise) return refreshPromise;
    refreshPromise = (async () => {
      for (const wc of electron.webContents.getAllWebContents()) track(wc);
      for (const record of windows.values()) {
        const wc = record.wc;
        if (wc.isDestroyed()) { windows.delete(wc.id); continue; }
        const workbench = /\/vs\/(?:code\/[^/]+\/)?workbench\/workbench(?:\.[^/?#]+)?\.html(?:[?#]|$)/;
        if (!workbench.test(wc.getURL())) { record.phase = "ignored"; continue; }
        await refreshPrimary(record);
        for (const guard of guards) await refreshGuard(record, guard);
        if (config.isolationHost) await refreshIsolation(record);
      }
    })().finally(() => { refreshPromise = undefined; });
    return refreshPromise;
  }

  async function refreshPrimary(record) {
    if (record.retryAt > Date.now()) return;
    const wc = record.wc;
    const generation = record.generation;
    try {
      let state = await readState(wc);
      if (!state) state = await install(wc, primary);
      if (stopped || !enabled || generation !== record.generation) {
        if (stopped || !enabled) await restoreWindow(wc).catch(() => {});
        return;
      }
      if (!state.enabled || !state.intact || !state.verified) throw new Error("The completion adapter is not verified or was replaced");
      record.phase = "active";
      record.surface = state.surface;
      record.suppressed = state.suppressed;
      record.verified = state.verified;
      record.checks = state.checks;
      record.attempts = 0;
      record.retryAt = 0;
      delete record.error;
    } catch (error) {
      if (generation !== record.generation) return;
      record.phase = "error";
      record.error = String(error.message ?? error).slice(0, 200);
      record.attempts++;
      record.retryAt = Date.now() + Math.min(30000, 1000 * 2 ** Math.min(record.attempts - 1, 5));
    }
  }

  // An extra guard keeps its own retry schedule: its failure never changes the primary's phase,
  // and a settled "unavailable" is only polled, not reinstalled, until the window reloads.
  async function refreshGuard(record, guard) {
    if (stopped || !enabled) return;
    const slot = record.guards[guard.name] ??= { attempts: 0, retryAt: 0 };
    if (slot.retryAt > Date.now()) return;
    const wc = record.wc;
    const generation = record.generation;
    try {
      let state = await readState(wc, guard.key);
      if (!state) state = await install(wc, guard);
      if (guard.name === "stall" && state.state === "pending") {
        slot.waitSince ??= Date.now();
        if (state.hostPid) {
          attachStallHost ??= eval(config.stallAttach)(require, config, bounded);
          const { pid, token, retryWindowMs } = await attachStallHost(state.hostPid);
          if (!stopped && enabled && generation === record.generation) {
            state = await evaluate(wc, `(() => {
              const s = globalThis[${JSON.stringify(guard.key)}];
              if (s?.lease !== ${JSON.stringify(lease)}) return null;
              return s.setHost(${pid}, ${JSON.stringify(token)}, ${JSON.stringify(retryWindowMs)});
            })()`);
          }
        } else if (Date.now() - slot.waitSince >= 10000) {
          state = { state: "unavailable", error: "Cursor has no verified local Agent Host for this window" };
        }
      } else {
        delete slot.waitSince;
      }
      if (stopped || !enabled || generation !== record.generation) {
        if (stopped || !enabled) await restoreWindow(wc).catch(() => {});
        return;
      }
      if (!guard.ready(state)) throw new Error(guard.notReady);
      slot.status = state;
      slot.attempts = 0;
      slot.retryAt = 0;
      delete slot.error;
    } catch (error) {
      if (generation !== record.generation) return;
      slot.error = String(error.message ?? error).slice(0, 200);
      slot.attempts++;
      slot.retryAt = Date.now() + Math.min(30000, 1000 * 2 ** Math.min(slot.attempts - 1, 5));
    }
  }

  async function refreshIsolation(record) {
    if (stopped || !enabled) return;
    const stall = record.guards.stall?.status;
    const pid = stall?.hostPid;
    const extension = stall?.hostExtension === "cursor-always-local" ? "cursor-always-local" : "cursor-agent-host";
    if (!Number.isSafeInteger(pid)) {
      record.isolationWaitSince ??= Date.now();
      record.isolation = stall?.state === "unavailable" || record.guards.stall?.error ||
        Date.now() - record.isolationWaitSince >= 10000
        ? {state: "unavailable", error: "Cursor has no verified local Agent Host"} : {state: "pending"};
      return;
    }
    delete record.isolationWaitSince;
    const prior = isolationHosts.get(pid);
    if (prior && (prior.checkAt > Date.now() || prior.retryAt > Date.now())) {
      record.isolation = prior.status;
      return;
    }
    const generation = record.generation;
    try {
      attachStallHost ??= eval(config.stallAttach)(require, config, bounded);
      if (prior?.checkAt && prior.identity === await attachStallHost.identity(pid)) {
        prior.checkAt = Date.now() + 15000;
        record.isolation = prior.status;
        return;
      }
      const attached = await attachStallHost.isolate(pid, lease, identity => {
        isolationHosts.set(pid, {identity, status: {state: "pending"}});
      }, extension);
      attached.checkAt = Date.now() + 15000;
      isolationHosts.set(pid, attached);
      if (generation === record.generation) record.isolation = attached.status;
    } catch (error) {
      const status = {state: "unavailable", error: String(error.message ?? error).slice(0, 180)};
      isolationHosts.set(pid, {identity: isolationHosts.get(pid)?.identity, status, retryAt: Date.now() + 30000});
      if (generation === record.generation) record.isolation = status;
    }
  }

  // Only known, bounded fields leave the window. Counts are plain non-negative integers that the
  // daemon can always parse: 64-bit fields stay within the safe range, 32-bit ones within a million.
  const count = value => Number.isFinite(value) && value > 0 ? Math.min(Math.floor(value), Number.MAX_SAFE_INTEGER) : 0;
  const small = value => Math.min(count(value), 1000000);
  const guardStatus = (record, guard) => {
    const slot = record.guards[guard.name] ?? {};
    const reported = slot.status ?? {};
    const error = slot.error ?? (typeof reported.error === "string" ? reported.error.slice(0, 200) : undefined);
    const state = slot.error ? "unavailable" : typeof reported.state === "string" ? reported.state : "pending";
    if (guard.name === "stall") {
      return {
        state, error,
        timeoutMs: Number.isSafeInteger(reported.timeoutMs) ? reported.timeoutMs : undefined,
        retryWindowMs: Number.isSafeInteger(reported.retryWindowMs) ? reported.retryWindowMs : undefined,
      };
    }
    return {
      state,
      error, checks: small(reported.checks),
      skipped: count(reported.skipped), skippedBubbles: count(reported.skippedBubbles), passed: count(reported.passed),
      lastSkippedAt: count(reported.lastSkippedAt),
      recent: (Array.isArray(reported.recent) ? reported.recent : []).slice(-20).map(entry => ({
        at: count(entry?.at), bubbles: count(entry?.bubbles), scope: entry?.scope === "all" ? "all" : "partial",
      })),
      atRisk: Number.isFinite(reported.atRisk) ? small(reported.atRisk) : null, atRiskAt: count(reported.atRiskAt),
    };
  };

  const status = () => ({
    protocol: 1, adapterRevision: config.revision, pid: process.pid, version: electron.app.getVersion(), enabled: enabled && !stopped,
    windows: [...windows.values()].filter(r => r.phase !== "ignored" && !r.wc.isDestroyed()).map(r => ({
      id: r.wc.id, phase: r.phase, surface: r.surface, suppressed: r.suppressed,
      verified: r.phase === "active" && r.verified === true, checks: r.checks ?? 0, error: r.error,
      ...(config.isolationHost ? {isolation: r.isolation ?? {state: "pending"}} : {}),
      ...Object.fromEntries(guards.map(guard => [guard.name, guardStatus(r, guard)])),
    })),
  });
  const created = (_event, wc) => track(wc);
  const timer = setInterval(() => { void refresh().catch(() => {}); }, 2000);
  timer.unref();
  electron.app.on("web-contents-created", created);
  let server;
  async function restore() {
    enabled = false;
    stopped = true;
    clearInterval(timer);
    electron.app.removeListener("web-contents-created", created);
    const deadline = Date.now() + 4000;
    try {
      if (refreshPromise) await bounded(refreshPromise, 3000, "Attachment teardown is pending");
      if (attachStallHost && isolationHosts.size) {
        const hosts = [...isolationHosts].filter(([, state]) => state.identity);
        const restored = await bounded(Promise.all(hosts.map(([pid, state]) =>
          attachStallHost.restoreIsolation(pid, state.identity, lease))),
          Math.max(1, deadline - Date.now()), "HTTP/2 isolation restoration is pending");
        if (!restored.every(Boolean)) return {restored: false};
        isolationHosts.clear();
      }
      const results = await bounded(Promise.all([...windows.values()].map(async record => {
        record.generation++;
        record.wc.removeListener("did-start-loading", record.reload);
        record.wc.removeListener("destroyed", record.destroy);
        if (record.wc.isDestroyed()) return true;
        return restoreWindow(record.wc).catch(() => false);
      })), Math.max(1, deadline - Date.now()), "Window restoration is pending");
      if (!results.every(Boolean)) return { restored: false };
      server?.close();
      if (globalThis[key] === registration) delete globalThis[key];
      return { restored: true };
    } catch {
      // Keep authenticated control available for a retry instead of orphaning a live guard.
      return { restored: false };
    }
  }

  server = net.createServer(socket => {
    let buffer = "";
    socket.setTimeout(5000, () => socket.destroy());
    socket.setEncoding("utf8");
    socket.on("error", () => {});
    socket.on("data", chunk => {
      buffer += chunk;
      if (Buffer.byteLength(buffer) > 4096) { socket.destroy(); return; }
      const end = buffer.indexOf("\n");
      if (end < 0) return;
      socket.pause();
      void (async () => {
        try {
          const request = JSON.parse(buffer.slice(0, end));
          const supplied = Buffer.from(typeof request.token === "string" ? request.token : "");
          const expected = Buffer.from(config.token);
          if (supplied.length !== expected.length || !crypto.timingSafeEqual(supplied, expected)) {
            throw new Error("Authentication failed");
          }
          let result;
          if (request.op === "status") result = status();
          else if (request.op === "restore") result = await restore();
          else throw new Error("Unknown bridge operation");
          socket.end(JSON.stringify({ ok: true, result }) + "\n");
        } catch (error) {
          socket.end(JSON.stringify({ ok: false, error: String(error.message ?? error).slice(0, 200) }) + "\n");
        }
      })();
    });
  });
  server.maxConnections = 16;
  try {
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(config.socket, resolve);
    });
    fs.chmodSync(config.socket, 0o600);
    server.on("error", () => {});
    server.unref();
    registration = { token: config.token, protocol: 1, revision: config.revision, status, restore };
    globalThis[key] = registration;
    void refresh().catch(() => {});
    return status();
  } catch (error) {
    await restore();
    throw error;
  }
})()
