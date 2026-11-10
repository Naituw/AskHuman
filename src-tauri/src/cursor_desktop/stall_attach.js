((require, config, bounded) => {
  "use strict";
  const { execFile } = process.getBuiltinModule("child_process");
  const { dirname, join } = process.getBuiltinModule("path");
  const http = process.getBuiltinModule("http");
  const WebSocket = require("ws");
  const hostEntry = join(dirname(config.appPackage), "extensions", "cursor-agent-host", "dist", "main.js");
  const inspected = new Map();
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
  const output = (file, args) => new Promise((resolve, reject) => {
    execFile(file, args, { timeout: 1200, maxBuffer: 16384 }, (error, stdout, stderr) => {
      if (!error || (error.code === 1 && !stderr && !stdout)) resolve(stdout);
      else reject(new Error("Could not verify the local Agent Host process"));
    });
  });

  async function identity(pid) {
    if (!Number.isSafeInteger(pid) || pid <= 0) return undefined;
    const result = (await output("/bin/ps", ["-p", String(pid), "-o", "ppid=,uid=,lstart=,comm="])).trim();
    const fields = result.split(/\s+/, 3);
    if (fields[0] !== String(process.pid) || fields[1] !== String(process.getuid()) ||
        !result.includes("Cursor Helper (Plugin): extension-host ")) return undefined;
    return result;
  }

  async function listeners(pid) {
    const text = await output("/usr/sbin/lsof", ["-nP", "-a", "-p", String(pid), "-iTCP", "-sTCP:LISTEN", "-Fn"]);
    return text.split("\n").filter(line => line.startsWith("n")).map(line => line.slice(1));
  }

  function debuggerTarget(port) {
    return new Promise((resolve, reject) => {
      const req = http.get({
        host: "127.0.0.1", port, path: "/json/list", agent: false, timeout: 900,
      }, res => {
        let body = "";
        res.setEncoding("utf8");
        res.on("error", reject);
        res.on("data", chunk => {
          body += chunk;
          if (body.length > 16384) req.destroy(new Error("Agent Host inspector target list is too large"));
        });
        res.on("end", () => {
          try {
            const targets = JSON.parse(body);
            if (res.statusCode !== 200 || targets.length !== 1 || targets[0].type !== "node") {
              throw new Error("Agent Host inspector has an unexpected target");
            }
            const url = new URL(targets[0].webSocketDebuggerUrl);
            if (url.origin !== `ws://127.0.0.1:${port}` || url.username || url.password ||
                !/^\/[\da-f-]{36}$/.test(url.pathname) || url.search || url.hash) {
              throw new Error("Agent Host inspector target is outside the verified loopback listener");
            }
            resolve(url);
          } catch (error) { reject(error); }
        });
      });
      req.on("error", reject);
      req.on("timeout", () => req.destroy(new Error("Agent Host inspector target timed out")));
    });
  }

  async function connect(port) {
    const socket = new WebSocket(await debuggerTarget(port), { handshakeTimeout: 900, maxPayload: 2 * 1024 * 1024 });
    const pending = new Map();
    let serial = 0, onPause;
    const terminate = error => {
      for (const request of pending.values()) request.reject(error);
      pending.clear();
    };
    socket.on("error", terminate);
    socket.on("close", () => terminate(new Error("Agent Host inspector disconnected")));
    socket.on("message", bytes => {
      let message;
      try { message = JSON.parse(bytes.toString()); }
      catch { socket.terminate(); return; }
      if (message.method === "Debugger.paused") onPause?.(message.params);
      const request = pending.get(message.id);
      if (!request) return;
      pending.delete(message.id);
      if (message.error || message.result?.exceptionDetails) {
        request.reject(new Error(`Agent Host inspector rejected ${request.method}`));
      } else request.resolve(message.result);
    });
    try {
      await bounded(new Promise((resolve, reject) => {
        socket.once("open", resolve);
        socket.once("error", reject);
      }), 1000, "Agent Host inspector handshake timed out");
    } catch (error) {
      socket.terminate();
      throw error;
    }
    function send(method, params = {}) {
      const id = ++serial;
      return bounded(new Promise((resolve, reject) => {
        if (socket.readyState !== WebSocket.OPEN) return reject(new Error("Agent Host inspector is closed"));
        pending.set(id, { method, resolve, reject });
        socket.send(JSON.stringify({ id, method, params }), error => { if (error) reject(error); });
      }), 3000, `Agent Host inspector ${method} timed out`).finally(() => pending.delete(id));
    }
    const evaluate = (expression, returnByValue = true) => send("Runtime.evaluate", {
      expression, returnByValue,
    }).then(reply => returnByValue ? reply.result?.value : reply.result?.objectId);
    const properties = objectId => send("Runtime.getProperties", { objectId, ownProperties: true });
    async function frame(cache, moduleId, key, expression) {
      const { result } = await send("Runtime.callFunctionOn", {
        objectId: cache, functionDeclaration: "function(id) { return this[id]?.exports; }",
        arguments: [{ value: moduleId }],
      });
      const target = result?.objectId;
      if (!target) throw new Error("Cursor has not loaded the Agent Host retry modules");
      const getter = (await properties(target)).result.find(value => value.name === key)?.get?.objectId;
      if (!getter) throw new Error("Unsupported Agent Host module export");
      let breakpoint, oursPaused = false;
      try {
        await send("Debugger.enable");
        breakpoint = (await send("Debugger.setBreakpointOnFunctionCall", { objectId: getter })).breakpointId;
        const paused = new Promise((resolve, reject) => {
          onPause = async event => {
            if (!event.hitBreakpoints?.includes(breakpoint)) return;
            oursPaused = true;
            try {
              const reply = await send("Debugger.evaluateOnCallFrame", {
                callFrameId: event.callFrames[0].callFrameId, expression, returnByValue: true,
              });
              resolve(reply.result?.value);
            } catch (error) { reject(error); }
            finally {
              if (breakpoint) {
                await send("Debugger.removeBreakpoint", { breakpointId: breakpoint }).catch(() => {});
                breakpoint = undefined;
              }
              await send("Debugger.resume").then(() => { oursPaused = false; }).catch(() => {});
            }
          };
        });
        const invoke = send("Runtime.callFunctionOn", {
          objectId: target, functionDeclaration: "function(key) { void this[key]; }", arguments: [{ value: key }],
        });
        return (await bounded(Promise.all([paused, invoke]), 5000, "Agent Host native getter was not reached"))[0];
      } finally {
        onPause = undefined;
        if (breakpoint) await send("Debugger.removeBreakpoint", { breakpointId: breakpoint }).catch(() => {});
        if (oursPaused) await send("Debugger.resume").catch(() => {});
        await send("Debugger.disable").catch(() => {});
      }
    }
    return { evaluate, discover: eval(config.stallDiscovery)(send).host, frame, close: () => socket.terminate() };
  }

  const closeCommand = "setTimeout(() => process.getBuiltinModule('inspector').close(), 30).unref(); true";
  async function withInspector(pid, port, job) {
    const peer = await connect(port);
    let verified = false;
    try {
      verified = await peer.evaluate("process.pid") === pid;
      if (!verified) throw new Error("Agent Host inspector PID changed");
      const watchdog = await peer.evaluate(
        "Number(setTimeout(() => process.getBuiltinModule('inspector').close(), 12000).unref())");
      try { return await job(peer); }
      finally {
        await peer.evaluate(`clearTimeout(${Number(watchdog)}); ${closeCommand}`).catch(() => {});
      }
    } finally {
      if (verified) await peer.evaluate(closeCommand).catch(() => {});
      peer.close();
    }
  }

  async function attach(pid) {
    const before = await identity(pid);
    if (!before) throw new Error("Cursor Agent Host identity could not be verified");
    const cached = inspected.get(pid);
    if (cached?.identity === before) return cached.result;
    // Another debugger or native profiler owns any listener that was already present.
    if ((await listeners(pid)).length) throw new Error("Cursor Agent Host already has a TCP listener");
    if (await identity(pid) !== before) throw new Error("Cursor Agent Host was replaced");
    process._debugProcess(pid);
    let port, result, failure;
    try {
      for (let attempt = 0; attempt < 8; attempt++) {
        const open = await listeners(pid);
        if (open.length) {
          const loopback = open.length === 1 && /^127\.0\.0\.1:(\d+)$/.exec(open[0]);
          if (!loopback) throw new Error("Agent Host inspector did not open a private loopback port");
          port = Number(loopback[1]);
          break;
        }
        await sleep(75);
      }
      if (!port || await identity(pid) !== before) throw new Error("Agent Host inspector did not start");
      result = await withInspector(pid, port, async peer => {
        const activate = await peer.evaluate(
          `process.getBuiltinModule("module")._cache[${JSON.stringify(hostEntry)}]?.exports?.activate`, false);
        if (!activate) throw new Error("Cursor has not loaded the Agent Host extension");
        const installer = `(${config.stallHost})(${JSON.stringify(config.owner)},${JSON.stringify(config.revision)})`;
        const prior = await peer.evaluate(`(() => {
          const state = globalThis.__ASKHUMAN_CURSOR_STALL_HOST_V1__;
          return state?.owner === ${JSON.stringify(config.owner)} && state?.revision === ${JSON.stringify(config.revision)}
            ? state.status?.() : undefined;
        })()`);
        if (prior?.pid === pid) return prior;
        const { cache, runner, classifier, decoder } = await peer.discover(activate);
        if (await peer.frame(cache, runner.id, runner.key,
          `${installer}.attachRunner(() => ${runner.name}, v => { ${runner.name} = v; }, ${classifier.name}, ${classifier.cutoff})`) !== true) {
          throw new Error("Cursor Agent Host retry hook did not pass verification");
        }
        return peer.frame(cache, decoder.id, decoder.key, `${installer}.attachDecoder(${decoder.name}.stallDetectorConfig.turn)`);
      });
    } catch (error) { failure = error; }
    if (port && await identity(pid) === before) {
      for (let attempt = 0; attempt < 6; attempt++) {
        if (!(await listeners(pid)).includes(`127.0.0.1:${port}`)) break;
        if (attempt === 3) {
          await withInspector(pid, port, async () => {}).catch(() => {});
        }
        await sleep(100);
      }
      if ((await listeners(pid)).includes(`127.0.0.1:${port}`)) {
        throw new Error("Temporary Cursor Agent Host inspector did not close");
      }
    }
    if (failure) throw failure;
    if (result?.pid !== pid || !/^[0-9a-f]{32}$/.test(result?.token) || await identity(pid) !== before) {
      throw new Error("Agent Host attachment did not verify");
    }
    // Retain only recent Host identities. The native lifetime is still tied to the window.
    if (inspected.size >= 64) inspected.delete(inspected.keys().next().value);
    inspected.set(pid, { identity: before, result });
    return result;
  }
  return attach;
})
