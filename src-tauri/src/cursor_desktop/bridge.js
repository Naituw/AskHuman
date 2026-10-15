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
  let enabled = true, stopped = false, refreshPromise, registration;
  const bounded = (promise, ms, label) => {
    let timer;
    return Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(label)), ms);
    })]).finally(() => clearTimeout(timer));
  };
  const evaluate = (wc, expression) => bounded(wc.executeJavaScript(expression), 3000, "Window evaluation timed out");
  const readState = wc => evaluate(wc, `(async () => {
    const s = globalThis[${JSON.stringify(rendererKey)}];
    if (!s) return null;
    if (s.owner !== ${JSON.stringify(config.owner)}) throw new Error("Another AskHuman instance owns this window");
    if (s.revision !== ${JSON.stringify(config.revision)}) throw new Error("A different adapter version requires restoration");
    if (s.lease !== ${JSON.stringify(lease)}) throw new Error("A different adapter installation requires restoration");
    return await s.activate();
  })()`);
  const restoreWindow = wc => evaluate(wc, `(() => {
    const s = globalThis[${JSON.stringify(rendererKey)}];
    if (!s) return true;
    if (s.owner !== ${JSON.stringify(config.owner)} || s.revision !== ${JSON.stringify(config.revision)} || s.lease !== ${JSON.stringify(lease)}) return false;
    return s.restore();
  })()`);

  async function install(wc) {
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
            const result = await command("Debugger.evaluateOnCallFrame", {
              callFrameId: frame.callFrameId, expression: installSource, returnByValue: true,
            });
            if (result.exceptionDetails) {
              const detail = result.exceptionDetails.exception?.description?.split("\n")[0];
              throw new Error(detail?.slice(0, 180) || "Completion adapter contract check failed");
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
      if (!state?.prepared) throw new Error("Completion adapter was not prepared");
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
    const state = await readState(wc);
    if (!state?.enabled || !state.intact || !state.verified) throw new Error("Completion adapter did not pass native preflight");
    return state;
  }

  function track(wc) {
    if (wc.isDestroyed() || wc.getType() !== "window" || windows.has(wc.id)) return;
    const record = { wc, phase: "attaching", generation: 0, suppressed: 0, retryAt: 0, attempts: 0 };
    record.reload = () => {
      record.generation++;
      record.phase = "attaching";
      record.retryAt = 0;
      record.attempts = 0;
      delete record.error;
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
        if (record.retryAt > Date.now()) continue;
        const generation = record.generation;
        try {
          let state = await readState(wc);
          if (!state) state = await install(wc);
          if (stopped || !enabled || generation !== record.generation) {
            if (stopped || !enabled) await restoreWindow(wc).catch(() => {});
            continue;
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
          if (generation !== record.generation) continue;
          record.phase = "error";
          record.error = String(error.message ?? error).slice(0, 200);
          record.attempts++;
          record.retryAt = Date.now() + Math.min(30000, 1000 * 2 ** Math.min(record.attempts - 1, 5));
        }
      }
    })().finally(() => { refreshPromise = undefined; });
    return refreshPromise;
  }

  const status = () => ({
    protocol: 1, adapterRevision: config.revision, pid: process.pid, version: electron.app.getVersion(), enabled: enabled && !stopped,
    windows: [...windows.values()].filter(r => r.phase !== "ignored" && !r.wc.isDestroyed()).map(r => ({
      id: r.wc.id, phase: r.phase, surface: r.surface, suppressed: r.suppressed,
      verified: r.phase === "active" && r.verified === true, checks: r.checks ?? 0, error: r.error,
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
