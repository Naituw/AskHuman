import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import vm from "node:vm";
import { afterEach, test } from "node:test";

const renderer = fs.readFileSync(new URL("../src-tauri/src/cursor_desktop/renderer.js", import.meta.url), "utf8").replace("__ASKHUMAN_PREFLIGHT__", "globalThis.__verify");
const bridge = fs.readFileSync(new URL("../src-tauri/src/cursor_desktop/bridge.js", import.meta.url), "utf8");
const rendererKey = "__ASKHUMAN_CURSOR_BACKGROUND_V1__";
const resources = [];
afterEach(async () => {
  for (const dispose of resources.splice(0).reverse()) await dispose();
});

function fixture(surface = "agents") {
  const calls = { models: 0, native: 0, ack: [], results: [] };
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
  const data = {
    _environmentService: { isGlass: surface === "agents" },
    _instantiationService: { _services: { _entries: new Map([
      ["composerAgentService", agent], ["backgroundWorkService", background],
    ]) } },
  };
  const context = vm.createContext({ __data: data, setTimeout, clearTimeout, __verify: async () => ({ passed: true, checks: 6 }) });
  function evaluate(expression) {
    return vm.runInContext(`(function(){return ${expression};}).call(__data)`, context);
  }
  function prepare(owner = "test-owner", revision = "test-revision", lease = "test-lease") {
    return evaluate(renderer.replace("__ASKHUMAN_OWNER__", JSON.stringify(owner))
      .replace("__ASKHUMAN_REVISION__", JSON.stringify(revision))
      .replace("__ASKHUMAN_LEASE__", JSON.stringify(lease)));
  }
  async function install(...args) { prepare(...args); return context[rendererKey].activate(); }
  async function dispatch(id, status) {
    // Native dispatch applies task state before the guarded entry and ACKs a completed delivery.
    calls.results.push({ id, status });
    const result = await agent._runBackgroundTaskCompletionAction({}, { composerId: id }, { status });
    if (result === "completed") background.ack([id]);
    return result;
  }
  return { calls, agent, background, data, context, evaluate, prepare, install, dispatch };
}

test("global control preserves real outcomes and native ACK without invoking a model", async () => {
  const f = fixture();
  const original = Object.getPrototypeOf(f.agent)._runBackgroundTaskCompletionAction;
  await f.install();
  for (const [id, status] of [["one", "success"], ["two", "error"], ["three", "aborted"]]) {
    await f.dispatch(id, status);
  }
  assert.equal(f.calls.models, 0);
  assert.deepEqual(f.calls.ack, ["one", "two", "three"]);
  assert.deepEqual(f.calls.results.map(r => r.status), ["success", "error", "aborted"]);
  assert.equal(f.context[rendererKey].status().suppressed, 3);
  assert.equal(f.context[rendererKey].restore(), true);
  assert.equal(Object.getPrototypeOf(f.agent)._runBackgroundTaskCompletionAction, original);
  await f.dispatch("four", "success");
  assert.equal(f.calls.models, 1);
  assert.equal(f.context[rendererKey], undefined);
});

test("new service instances share the guard while other actions still work", async () => {
  const f = fixture("ide");
  await f.install();
  const laterAgent = new (Object.getPrototypeOf(f.agent).constructor)();
  assert.equal(await laterAgent._runBackgroundTaskCompletionAction(), "completed");
  assert.equal(f.calls.models, 0);
  await f.agent._runWakeupAction();
  assert.equal(f.calls.models, 1);
  assert.equal(f.context[rendererKey].status().surface, "ide");
  f.context[rendererKey].restore();
});

test("attachment is idempotent, rejects another owner, and does not overwrite a later wrapper", async () => {
  const f = fixture();
  await f.install();
  const own = Object.getPrototypeOf(f.agent)._runBackgroundTaskCompletionAction;
  await f.install();
  assert.equal(Object.getPrototypeOf(f.agent)._runBackgroundTaskCompletionAction, own);
  await assert.rejects(() => f.install("other-owner"), /Another AskHuman/);
  await assert.rejects(() => f.install("test-owner", "next-version"), /different adapter version/);
  const later = function (...args) { return own.apply(this, args); };
  Object.getPrototypeOf(f.agent)._runBackgroundTaskCompletionAction = later;
  assert.equal(f.context[rendererKey].restore(), false);
  assert.equal(Object.getPrototypeOf(f.agent)._runBackgroundTaskCompletionAction, later);
  // A retained wrapper delegates to native behavior after our state has been disabled.
  await f.agent._runBackgroundTaskCompletionAction();
  assert.equal(f.calls.models, 1);
});

test("an unknown native contract is rejected before mutation", async () => {
  const f = fixture();
  const prototype = Object.getPrototypeOf(f.agent);
  prototype._runBackgroundTaskCompletionAction = async function changed() {};
  const before = prototype._runBackgroundTaskCompletionAction;
  await assert.rejects(() => f.install(), /Unsupported background completion contract/);
  assert.equal(prototype._runBackgroundTaskCompletionAction, before);
  assert.equal(f.context[rendererKey], undefined);
});

test("a dispatcher without native ACK bookkeeping is rejected", async () => {
  const f = fixture();
  Object.getPrototypeOf(f.agent)._maybeDispatchBackgroundCompletions = async function changed() {};
  await assert.rejects(() => f.install(), /Unsupported background completion contract/);
  assert.equal(f.context[rendererKey], undefined);
});

test("the legacy direct-ACK contract is accepted only without a paired inbox method", async () => {
  const f = fixture(), prototype = Object.getPrototypeOf(f.agent);
  prototype._maybeDispatchBackgroundCompletions = vm.runInNewContext('(async function(){this._pendingAwaitTaskIdsByComposerId;if(await this._runBackgroundTaskCompletionAction()==="completed")this._backgroundWorkService.ack([])})');
  await assert.rejects(f.install(), /Unsupported background completion contract/);
  delete prototype._commitPairedCloudChildInboxCursor;
  assert.equal((await f.install()).verified, true);
  f.context[rendererKey].restore();
});

class FakeWindow extends EventEmitter {
  constructor(id, surface) {
    super();
    this.id = id;
    this.surface = surface;
    this.f = fixture(surface);
    this.commands = [];
    this.attached = false;
    this.devTools = false;
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
  isDevToolsOpened() { return this.devTools; }
  executeJavaScript(expression) {
    if (expression.includes("await s.activate()")) assert.equal(this.attached, false, "preflight must run after debugger detach");
    return Promise.resolve(this.f.evaluate(expression));
  }
  reload() {
    this.f = fixture(this.surface);
    this.emit("did-start-loading");
  }
}

async function startBridge(windows) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ah-cd-"));
  fs.chmodSync(dir, 0o700);
  const config = { owner: "test-owner", revision: "test-revision", token: "a".repeat(64), socket: path.join(dir, "b.sock"), appPackage: "/fake/package.json" };
  const app = new EventEmitter();
  app.getVersion = () => "test";
  const electron = { app, webContents: { getAllWebContents: () => windows } };
  const realProcess = process;
  const context = vm.createContext({
    Buffer, setTimeout, clearTimeout, setInterval, clearInterval,
    process: {
      pid: process.pid,
      getBuiltinModule(name) {
        if (name === "module") return { createRequire: () => () => electron };
        return realProcess.getBuiltinModule(name);
      },
    },
  });
  const sourceFor = next => bridge
    .replace("__ASKHUMAN_RENDERER__", JSON.stringify(renderer.replace("__ASKHUMAN_OWNER__", JSON.stringify(next.owner))
      .replace("__ASKHUMAN_REVISION__", JSON.stringify(next.revision))))
    .replace("__ASKHUMAN_CONFIG__", JSON.stringify(next));
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
  resources.push(async () => {
    const live = context[`__ASKHUMAN_CURSOR_BRIDGE_${config.owner}__`];
    if (live) await live.restore();
    fs.rmSync(dir, { recursive: true, force: true });
  });
  return { request, config, context, app, windows, rebootstrap };
}

async function until(check) {
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  assert.fail("Condition did not become true");
}

test("private bridge authenticates requests, limits operations, and restores both window types", async () => {
  const windows = [new FakeWindow(1, "agents"), new FakeWindow(2, "ide")];
  const b = await startBridge(windows);
  await until(async () => (await b.request("status")).result.windows.every(w => w.phase === "active"));
  assert.equal(fs.statSync(b.config.socket).mode & 0o777, 0o600);
  assert.equal((await b.request("restore", "wrong")).ok, false);
  assert.equal((await b.request("evaluate")).ok, false);
  for (const wc of windows) {
    await wc.f.dispatch("task", "error");
    assert.equal(wc.f.calls.models, 0);
    assert.equal(wc.attached, false);
    assert.equal(wc.listenerCount("message"), 0);
  }
  assert.equal((await b.request("restore")).result.restored, true);
  for (const wc of windows) {
    await wc.f.dispatch("later", "success");
    assert.equal(wc.f.calls.models, 1);
    assert.equal(wc.listenerCount("did-start-loading"), 0);
  }
});

test("new windows and reloaded contexts receive a new guard without layering wrappers", async () => {
  const wc = new FakeWindow(1, "agents");
  const b = await startBridge([wc]);
  await until(() => wc.f.context[rendererKey]?.status().enabled === true);
  wc.reload();
  const later = new FakeWindow(2, "ide");
  b.windows.push(later);
  b.app.emit("web-contents-created", {}, later);
  await until(() => wc.f.context[rendererKey]?.status().enabled === true && later.f.context[rendererKey]?.status().enabled === true);
  await wc.f.dispatch("reloaded", "success");
  await later.f.dispatch("new-window", "success");
  assert.equal(wc.f.calls.models, 0);
  assert.equal(later.f.calls.models, 0);
  assert.equal(wc.f.context[rendererKey].suppressed, 1);
  assert.equal(later.f.context[rendererKey].suppressed, 1);
});

test("a user debugger is left intact and incomplete coverage is reported", async () => {
  const wc = new FakeWindow(1, "ide");
  wc.devTools = true;
  const b = await startBridge([wc]);
  await until(async () => (await b.request("status")).result.windows[0]?.phase === "error");
  assert.equal(wc.commands.length, 0);
  assert.equal(wc.f.context[rendererKey], undefined);
  assert.match((await b.request("status")).result.windows[0].error, /debugger is in use/);
  assert.equal(wc.devTools, true);
});

test("a failed restoration keeps authenticated control available until retry succeeds", async () => {
  const wc = new FakeWindow(1, "agents");
  const b = await startBridge([wc]);
  await until(() => wc.f.context[rendererKey]?.status().enabled === true);
  const execute = wc.executeJavaScript.bind(wc);
  wc.executeJavaScript = expression => expression.includes("return s.restore()")
    ? Promise.reject(new Error("Window is temporarily unavailable")) : execute(expression);
  assert.equal((await b.request("restore")).result.restored, false);
  assert.equal((await b.request("status")).ok, true);
  wc.executeJavaScript = execute;
  assert.equal((await b.request("restore")).result.restored, true);
  assert.equal(wc.f.context[rendererKey], undefined);
});

test("bootstrap recovers a lost credential and stale callbacks cannot remove the new installation", async () => {
  const wc = new FakeWindow(1, "agents");
  const b = await startBridge([wc]);
  await until(() => wc.f.context[rendererKey]?.status().enabled === true);
  const lease = wc.f.context[rendererKey].lease;
  const staleRestore = b.context[`__ASKHUMAN_CURSOR_BRIDGE_${b.config.owner}__`].restore;
  const token = "b".repeat(64);
  await b.rebootstrap(token);
  await until(() => wc.f.context[rendererKey]?.lease !== lease && wc.f.context[rendererKey]?.status().enabled === true);
  assert.equal((await b.request("status")).ok, false);
  assert.equal((await b.request("status", token)).ok, true);
  assert.equal((await staleRestore()).restored, false);
  assert.equal((await b.request("status", token)).ok, true);
  await wc.f.dispatch("after-recovery", "success");
  assert.equal(wc.f.calls.models, 0);
  assert.equal((await b.request("restore", token)).result.restored, true);
});

test("pending verification never changes the native handler and cannot activate after restore", async () => {
  const f = fixture(), prototype = Object.getPrototypeOf(f.agent), original = prototype._runBackgroundTaskCompletionAction;
  let finish;
  f.context.__verify = () => new Promise(resolve => { finish = resolve; });
  const prepared = f.prepare();
  assert.equal(prepared.verified, false);
  const state = f.context[rendererKey], pending = state.activate();
  assert.equal(prototype._runBackgroundTaskCompletionAction, original);
  assert.equal(state.restore(), true);
  finish({ passed: true, checks: 6 });
  await assert.rejects(pending, /cancelled/);
  assert.equal(prototype._runBackgroundTaskCompletionAction, original);
  assert.equal(f.context[rendererKey], undefined);
});

test("a handler replaced during preflight is preserved", async () => {
  const f = fixture(), prototype = Object.getPrototypeOf(f.agent);
  let finish;
  f.context.__verify = () => new Promise(resolve => { finish = resolve; });
  f.prepare();
  const pending = f.context[rendererKey].activate(), replacement = async () => "foreign";
  prototype._runBackgroundTaskCompletionAction = replacement;
  finish({ passed: true, checks: 6 });
  await assert.rejects(pending, /changed during preflight/);
  assert.equal(prototype._runBackgroundTaskCompletionAction, replacement);
  assert.equal(f.context[rendererKey], undefined);
});

test("a stalled verifier times out without leaving a guard", async () => {
  const f = fixture(), original = Object.getPrototypeOf(f.agent)._runBackgroundTaskCompletionAction;
  f.context.__verify = () => new Promise(() => {});
  await assert.rejects(f.install(), /preflight timed out/);
  assert.equal(Object.getPrototypeOf(f.agent)._runBackgroundTaskCompletionAction, original);
  assert.equal(f.context[rendererKey], undefined);
});

test("failed native verification is reported as unavailable with no live mutation", async () => {
  const wc = new FakeWindow(1, "ide"), original = Object.getPrototypeOf(wc.f.agent)._runBackgroundTaskCompletionAction;
  wc.f.context.__verify = async () => { throw new Error("Native ACK failed"); };
  const b = await startBridge([wc]);
  await until(async () => (await b.request("status")).result.windows[0]?.phase === "error");
  const status = (await b.request("status")).result.windows[0];
  assert.equal(status.verified, false);
  assert.match(status.error, /Native ACK failed/);
  assert.equal(Object.getPrototypeOf(wc.f.agent)._runBackgroundTaskCompletionAction, original);
  assert.equal(wc.f.context[rendererKey], undefined);
});
