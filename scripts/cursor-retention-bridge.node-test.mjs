import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import vm from "node:vm";
import { afterEach, test } from "node:test";

// The bridge hosting both guards (completion + running-chat protection) in one window.
const read = name => fs.readFileSync(new URL(`../src-tauri/src/cursor_desktop/${name}`, import.meta.url), "utf8");
const renderer = read("renderer.js").replace("__ASKHUMAN_PREFLIGHT__", "globalThis.__verify");
const retention = read("retention.js");
const retentionPreflight = read("retention_preflight.js");
const bridge = read("bridge.js");
const completionKey = "__ASKHUMAN_CURSOR_BACKGROUND_V1__";
const retentionKey = "__ASKHUMAN_CURSOR_RETENTION_V1__";
const method = "unloadBubblesByIds";
const resources = [];
const bridgeTest = (name, run) => test(name, {
  skip: process.platform === "win32" ? "Unix socket bridge requires a Unix host" : false,
}, run);
afterEach(async () => {
  for (const dispose of resources.splice(0).reverse()) await dispose();
});

function fixture(surface = "agents") {
  const calls = { models: 0, native: 0, ack: [] };
  class Agent {
    async _runWakeupAction() { calls.models++; return "completed"; }
    async _runBackgroundTaskCompletionAction(...args) {
      calls.native++;
      return this._runWakeupAction(...args);
    }
    async _commitPairedCloudChildInboxCursor() {}
    async _maybeDispatchBackgroundCompletions() {
      this._pendingAwaitTaskIdsByComposerId;
      if (await this._runBackgroundTaskCompletionAction() === "completed") {
        this._backgroundWorkService.ack([]);
        await this._commitPairedCloudChildInboxCursor();
      }
    }
  }
  const agent = new Agent();
  const background = { ack(ids) { calls.ack.push(...ids); }, nack() {} };
  const natives = [];
  class Data {
    constructor() {
      this._environmentService = { isGlass: surface === "agents" };
      this._instantiationService = { _services: { _entries: new Map([
        ["composerAgentService", agent], ["backgroundWorkService", background],
      ]) } };
      this.handles = new Map();
      this.loadedComposers = { byId: {} };
    }
    getComposerData(handle) { try { return handle.data; } catch { return undefined; } }
    getHandleIfLoaded(id) { return this.handles.get(id); }
    getLastHumanBubbleId(handle) { return handle.data.headers.findLast(header => header.type === "human")?.bubbleId; }
    getLoadedConversation(handle) {
      const tail = [];
      for (let index = handle.data.headers.length - 1; index >= 0; index--) {
        const bubble = handle.data.conversationMap[handle.data.headers[index].bubbleId];
        if (!bubble) break;
        tail.push(bubble);
      }
      return tail.reverse();
    }
    unloadBubblesByIds(t, e, n) {
      natives.push({ t, e });
      const a = this.getComposerData(t);
      if (!a) return;
      const generating = new Set(a.generatingBubbleIds ?? []);
      for (const id of e) if (!generating.has(id) && a.conversationMap[id] != null) delete a.conversationMap[id];
    }
  }
  const data = new Data();
  const context = vm.createContext({ __data: data, setTimeout, clearTimeout, __verify: async () => ({ passed: true, checks: 6 }) });
  const evaluate = expression => vm.runInContext(`(function(){return ${expression};}).call(__data)`, context);
  const chat = (id, status = "completed") => {
    const headers = Array.from({ length: 6 }, (_, index) => ({ bubbleId: `${id}-${index}`, type: index === 0 ? "human" : "ai" }));
    const conversationMap = Object.fromEntries(headers.map(header => [header.bubbleId, { bubbleId: header.bubbleId }]));
    const handle = { composerId: id, data: { status, headers, conversationMap, generatingBubbleIds: [] } };
    data.handles.set(id, handle);
    data.loadedComposers.byId[id] = handle.data;
    return handle;
  };
  const loaded = handle => Object.keys(handle.data.conversationMap).length;
  const unload = (handle, count = 6) => data[method](handle, handle.data.headers.slice(0, count).map(header => header.bubbleId));
  return { calls, agent, data, natives, context, evaluate, chat, loaded, unload, prototype: Data.prototype, completion: Object.getPrototypeOf(agent) };
}

class FakeWindow extends EventEmitter {
  constructor(id, surface) {
    super();
    this.id = id;
    this.surface = surface;
    this.f = fixture(surface);
    this.commands = [];
    this.attached = false;
    this.debugger = {
      isAttached: () => this.attached,
      attach: () => { this.attached = true; },
      detach: () => { this.attached = false; },
      on: (...args) => this.on(...args),
      removeListener: (...args) => this.removeListener(...args),
      sendCommand: async (method, params) => {
        this.commands.push(method);
        if (method === "Runtime.evaluate" && params.expression.includes("?.getStates")) {
          return { result: { type: "function", objectId: "provider" } };
        }
        if (method === "Debugger.setBreakpointOnFunctionCall") return { breakpointId: "own" };
        if (method === "Runtime.evaluate" && params.expression.endsWith("getStates()")) {
          return new Promise(resolve => {
            this.resume = resolve;
            this.emit("message", {}, "Debugger.paused", {
              hitBreakpoints: ["own"], callFrames: [{ callFrameId: "frame" }],
            });
          });
        }
        if (method === "Debugger.evaluateOnCallFrame") {
          return { result: { value: await this.f.evaluate(params.expression) } };
        }
        if (method === "Debugger.resume") this.resume?.({ result: { value: [] } });
        return {};
      },
    };
  }
  isDestroyed() { return false; }
  getType() { return "window"; }
  getURL() { return "vscode-file://vscode-app/Applications/Cursor.app/Contents/Resources/app/out/vs/code/electron-sandbox/workbench/workbench.html"; }
  isDevToolsOpened() { return false; }
  executeJavaScript(expression) {
    if (expression.includes("await s.activate()")) assert.equal(this.attached, false, "activation must run after debugger detach");
    return Promise.resolve(this.f.evaluate(expression));
  }
  reload() {
    this.f = fixture(this.surface);
    this.emit("did-start-loading");
  }
  installs() { return this.commands.filter(command => command === "Debugger.setBreakpointOnFunctionCall").length; }
}

const config0 = () => ({
  owner: "test-owner", revision: "test-revision", token: "a".repeat(64), appPackage: "/fake/package.json",
});
const retentionSourceFor = next => retention
  .replace("__ASKHUMAN_OWNER__", () => JSON.stringify(next.owner))
  .replace("__ASKHUMAN_REVISION__", () => JSON.stringify(next.revision))
  .replace("__ASKHUMAN_RETENTION_PREFLIGHT__", () => retentionPreflight);

// Interval ticks are driven by hand so every refresh pass is deterministic.
async function startBridge(windows, { withRetention = true } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ah-rb-"));
  fs.chmodSync(dir, 0o700);
  const config = { ...config0(), socket: path.join(dir, "b.sock") };
  const app = new EventEmitter();
  app.getVersion = () => "test";
  const electron = { app, webContents: { getAllWebContents: () => windows } };
  const realProcess = process;
  const ticks = [];
  const context = vm.createContext({
    Buffer, setTimeout, clearTimeout,
    setInterval: fn => { ticks.push(fn); return { unref() {} }; }, clearInterval() {},
    process: {
      pid: process.pid,
      getBuiltinModule(name) {
        if (name === "module") return { createRequire: () => () => electron };
        return realProcess.getBuiltinModule(name);
      },
    },
  });
  const sourceFor = next => bridge
    .replace("__ASKHUMAN_RENDERER__", () => JSON.stringify(renderer.replace("__ASKHUMAN_OWNER__", JSON.stringify(next.owner))
      .replace("__ASKHUMAN_REVISION__", JSON.stringify(next.revision))))
    .replace("__ASKHUMAN_CONFIG__", () => JSON.stringify(withRetention
      ? { ...next, guards: [{ name: "retention", key: retentionKey, source: retentionSourceFor(next) }] } : next));
  const rebootstrap = (token, revision = config.revision) => vm.runInContext(sourceFor({ ...config, token, revision }), context);
  await rebootstrap(config.token);
  async function request(op, token = config.token) {
    const socket = net.createConnection(config.socket);
    socket.setTimeout(3000, () => socket.destroy(new Error("Request timeout")));
    socket.setEncoding("utf8");
    return new Promise((resolve, reject) => {
      let result = "";
      socket.on("error", reject);
      socket.on("connect", () => socket.write(JSON.stringify({ op, token }) + "\n"));
      socket.on("data", chunk => { result += chunk; });
      socket.on("end", () => resolve(JSON.parse(result)));
    });
  }
  const status = async () => (await request("status")).result;
  // Runs one more refresh pass and gives its (instant, fake) window operations time to finish.
  const pass = async () => {
    ticks.at(-1)();
    await new Promise(resolve => setTimeout(resolve, 25));
    await new Promise(resolve => setTimeout(resolve, 25));
  };
  resources.push(async () => {
    const live = context[`__ASKHUMAN_CURSOR_BRIDGE_${config.owner}__`];
    if (live) await live.restore();
    fs.rmSync(dir, { recursive: true, force: true });
  });
  return { request, status, pass, config, context, app, windows, rebootstrap };
}

async function until(check) {
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  assert.fail("Condition did not become true");
}
// The interval is driven by hand, so wait for a condition by running refresh passes.
async function eventually(b, check) {
  for (let round = 0; round < 40; round++) {
    await b.pass();
    if (await check()) return;
  }
  assert.fail("Condition did not become true");
}
const settled = windows => async () => windows.every(wc => wc.f.context[completionKey]?.status().enabled === true &&
  wc.f.context[retentionKey]?.status().state !== undefined && wc.f.context[retentionKey].status().state !== "pending");

bridgeTest("both guards attach in an Agents window and report independent status", async () => {
  const wc = new FakeWindow(1, "agents");
  const b = await startBridge([wc]);
  await until(settled([wc]));
  const [window] = (await b.status()).windows;
  assert.equal(window.phase, "active");
  assert.equal(window.verified, true);
  assert.equal(window.retention.state, "active");
  assert.ok(window.retention.checks >= 6);
  assert.equal(wc.attached, false);
  assert.equal(wc.listenerCount("message"), 0);

  const running = wc.f.chat("running", "generating");
  wc.f.unload(running);
  assert.equal(wc.f.loaded(running), 6);
  const idle = wc.f.chat("idle");
  wc.f.unload(idle, 3);
  assert.equal(wc.f.loaded(idle), 3);
  await wc.f.completion._runBackgroundTaskCompletionAction.call(wc.f.agent);
  assert.equal(wc.f.calls.models, 0, "the completion guard still suppresses the follow-up");

  await b.pass();
  const after = (await b.status()).windows[0];
  assert.equal(after.retention.skipped, 1);
  assert.equal(after.retention.skippedBubbles, 6);
  assert.equal(after.retention.passed, 1);
  assert.deepEqual(after.retention.recent.map(entry => entry.scope), ["all"]);
  assert.equal(after.suppressed, 1);
});

bridgeTest("an IDE window gets the completion guard and a not-applicable retention state", async () => {
  const wc = new FakeWindow(1, "ide");
  const original = wc.f.prototype[method];
  const b = await startBridge([wc]);
  await until(settled([wc]));
  const [window] = (await b.status()).windows;
  assert.equal(window.phase, "active");
  assert.equal(window.surface, "ide");
  assert.equal(window.retention.state, "notApplicable");
  assert.equal(wc.f.prototype[method], original);
});

bridgeTest("a changed unload contract leaves the completion guard active and is not retried", async () => {
  const wc = new FakeWindow(1, "agents");
  delete wc.f.prototype[method];
  const b = await startBridge([wc]);
  await until(settled([wc]));
  const first = (await b.status()).windows[0];
  assert.equal(first.phase, "active");
  assert.equal(first.verified, true);
  assert.equal(first.retention.state, "unavailable");
  assert.match(first.retention.error, /Unsupported conversation unload contract/);
  const installs = wc.installs();
  for (let round = 0; round < 3; round++) await b.pass();
  assert.equal(wc.installs(), installs, "a settled unavailable guard must not attach the debugger again");
  assert.equal((await b.status()).windows[0].retention.state, "unavailable");
  await wc.f.completion._runBackgroundTaskCompletionAction.call(wc.f.agent);
  assert.equal(wc.f.calls.models, 0);
});

bridgeTest("an unsupported completion contract does not stop the running-chat guard", async () => {
  const wc = new FakeWindow(1, "agents");
  wc.f.completion._runBackgroundTaskCompletionAction = async function changed() {};
  const b = await startBridge([wc]);
  await until(() => wc.f.context[retentionKey]?.status().state === "active");
  const window = (await b.status()).windows[0];
  assert.equal(window.phase, "error");
  assert.match(window.error, /Unsupported background completion contract/);
  assert.equal(window.retention.state, "active");
  const running = wc.f.chat("running", "generating");
  wc.f.unload(running);
  assert.equal(wc.f.loaded(running), 6);
});

bridgeTest("restoring puts back both Cursor methods exactly", async () => {
  const wc = new FakeWindow(1, "agents");
  const before = {
    unload: Object.getOwnPropertyDescriptor(wc.f.prototype, method),
    completion: Object.getOwnPropertyDescriptor(wc.f.completion, "_runBackgroundTaskCompletionAction"),
  };
  const b = await startBridge([wc]);
  await until(settled([wc]));
  assert.notEqual(wc.f.prototype[method], before.unload.value);
  assert.equal((await b.request("restore")).result.restored, true);
  assert.deepEqual(Object.getOwnPropertyDescriptor(wc.f.prototype, method), before.unload);
  assert.deepEqual(Object.getOwnPropertyDescriptor(wc.f.completion, "_runBackgroundTaskCompletionAction"), before.completion);
  assert.equal(wc.f.context[completionKey], undefined);
  assert.equal(wc.f.context[retentionKey], undefined);
  assert.equal(wc.listenerCount("did-start-loading"), 0);
});

bridgeTest("a retention restoration failure keeps authenticated control until a retry succeeds", async () => {
  const wc = new FakeWindow(1, "agents");
  const b = await startBridge([wc]);
  await until(settled([wc]));
  const execute = wc.executeJavaScript.bind(wc);
  wc.executeJavaScript = expression => expression.includes("return s.restore()") && expression.includes(retentionKey)
    ? Promise.reject(new Error("Window is temporarily unavailable")) : execute(expression);
  assert.equal((await b.request("restore")).result.restored, false);
  assert.equal((await b.request("status")).ok, true);
  wc.executeJavaScript = execute;
  assert.equal((await b.request("restore")).result.restored, true);
  assert.equal(wc.f.context[retentionKey], undefined);
  assert.equal(wc.f.context[completionKey], undefined);
});

bridgeTest("a later wrapper blocks restoration of the guard it covers without losing the other", async () => {
  const wc = new FakeWindow(1, "agents");
  const b = await startBridge([wc]);
  await until(settled([wc]));
  const ours = wc.f.prototype[method];
  const later = function (...args) { return ours.apply(this, args); };
  wc.f.prototype[method] = later;
  assert.equal((await b.request("restore")).result.restored, false);
  assert.equal(wc.f.prototype[method], later);
  assert.equal(wc.f.context[completionKey], undefined, "the independent guard was restored");
  assert.ok(wc.f.context[retentionKey], "the covered guard stays available for a retry");
  // Once the foreign wrapper is gone the retry can finish.
  wc.f.prototype[method] = ours;
  assert.equal((await b.request("restore")).result.restored, true);
});

bridgeTest("a reloaded window and a recovered bridge never layer wrappers", async () => {
  const wc = new FakeWindow(1, "agents");
  const b = await startBridge([wc]);
  await until(settled([wc]));
  wc.reload();
  const original = wc.f.prototype[method];
  await eventually(b, settled([wc]));
  assert.notEqual(wc.f.prototype[method], original, "the reloaded context was guarded again");
  const token = "b".repeat(64);
  await b.rebootstrap(token);
  await eventually(b, async () => (await b.request("status", token)).ok === true &&
    wc.f.context[retentionKey]?.status().state === "active");
  assert.equal((await b.request("restore", token)).result.restored, true);
  assert.equal(wc.f.prototype[method], original, "exactly one wrapper layer existed");
});

bridgeTest("a transient failure of one guard is retried with its own schedule", async () => {
  const wc = new FakeWindow(1, "agents");
  const execute = wc.executeJavaScript.bind(wc);
  let failures = 1;
  wc.executeJavaScript = expression => {
    if (failures > 0 && expression.includes(retentionKey) && expression.includes("await s.activate()")) {
      failures--;
      return Promise.reject(new Error("Window evaluation timed out"));
    }
    return execute(expression);
  };
  const b = await startBridge([wc]);
  await until(() => wc.f.context[completionKey]?.status().enabled === true);
  await until(async () => (await b.status()).windows[0]?.retention.state === "unavailable");
  const failed = (await b.status()).windows[0];
  assert.equal(failed.phase, "active", "the primary guard is unaffected");
  assert.match(failed.retention.error, /timed out/);
  await new Promise(resolve => setTimeout(resolve, 1100));
  await b.pass();
  const recovered = (await b.status()).windows[0];
  assert.equal(recovered.retention.state, "active");
  assert.equal(recovered.retention.error, undefined);
});

bridgeTest("only known, bounded, non-negative fields leave the window", async () => {
  const wc = new FakeWindow(1, "agents");
  const execute = wc.executeJavaScript.bind(wc);
  wc.executeJavaScript = async expression => {
    const value = await execute(expression);
    // Only a real (installed) state is altered; "not installed yet" must stay null.
    if (!value || !expression.includes(retentionKey) || !expression.includes("await s.activate()")) return value;
    return {
      ...value, secret: "composer-id", skipped: -4, skippedBubbles: Infinity, passed: "7", atRisk: Number.NaN,
      error: "x".repeat(500),
      recent: Array.from({ length: 60 }, (_, index) => ({ at: index, bubbles: -1, scope: "everything", composerId: "leak" })),
    };
  };
  const b = await startBridge([wc]);
  await until(async () => (await b.status()).windows[0]?.retention.recent.length === 20);
  const retentionStatus = (await b.status()).windows[0].retention;
  assert.deepEqual(Object.keys(retentionStatus).sort(),
    ["atRisk", "atRiskAt", "checks", "error", "lastSkippedAt", "passed", "recent", "skipped", "skippedBubbles", "state"]);
  assert.equal(retentionStatus.skipped, 0);
  assert.equal(retentionStatus.skippedBubbles, 0);
  assert.equal(retentionStatus.passed, 0);
  assert.equal(retentionStatus.atRisk, null);
  assert.equal(retentionStatus.error.length, 200);
  assert.deepEqual(Object.keys(retentionStatus.recent[0]).sort(), ["at", "bubbles", "scope"]);
  assert.ok(retentionStatus.recent.every(entry => entry.scope === "partial" && entry.bubbles === 0));
  assert.ok(!JSON.stringify(await b.status()).includes("composer-id"));
  assert.ok(!JSON.stringify(await b.status()).includes("leak"));
});

bridgeTest("huge numbers are capped to plain integers the daemon can always parse", async () => {
  const wc = new FakeWindow(1, "agents");
  const execute = wc.executeJavaScript.bind(wc);
  wc.executeJavaScript = async expression => {
    const value = await execute(expression);
    if (!value || !expression.includes(retentionKey) || !expression.includes("await s.activate()")) return value;
    return {
      ...value, checks: 1e300, skipped: 1e300, atRisk: 1e300, lastSkippedAt: 1e300,
      recent: [{ at: 1e300, bubbles: 1e300, scope: "all" }],
    };
  };
  const b = await startBridge([wc]);
  await until(async () => (await b.status()).windows[0]?.retention.skipped > 0);
  const retentionStatus = (await b.status()).windows[0].retention;
  assert.equal(retentionStatus.checks, 1000000);
  assert.equal(retentionStatus.atRisk, 1000000);
  assert.equal(retentionStatus.skipped, Number.MAX_SAFE_INTEGER);
  assert.equal(retentionStatus.lastSkippedAt, Number.MAX_SAFE_INTEGER);
  assert.deepEqual(retentionStatus.recent, [{ at: Number.MAX_SAFE_INTEGER, bubbles: Number.MAX_SAFE_INTEGER, scope: "all" }]);
  const numbers = [retentionStatus.checks, retentionStatus.atRisk, retentionStatus.skipped, retentionStatus.lastSkippedAt];
  assert.ok(numbers.every(Number.isInteger));
});

bridgeTest("without extra guards the bridge reports exactly what it reported before", async () => {
  const wc = new FakeWindow(1, "agents");
  const original = wc.f.prototype[method];
  const b = await startBridge([wc], { withRetention: false });
  await until(() => wc.f.context[completionKey]?.status().enabled === true);
  const [window] = (await b.status()).windows;
  assert.equal(window.phase, "active");
  assert.equal("retention" in window, false);
  assert.equal(wc.f.context[retentionKey], undefined);
  assert.equal(wc.f.prototype[method], original);
  assert.equal((await b.request("restore")).result.restored, true);
});
