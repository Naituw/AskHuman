import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { test } from "node:test";

const read = name => fs.readFileSync(new URL(`../src-tauri/src/cursor_desktop/${name}`, import.meta.url), "utf8");
const guardSource = read("retention.js");
const preflightSource = read("retention_preflight.js");
const key = "__ASKHUMAN_CURSOR_RETENTION_V1__";
const method = "unloadBubblesByIds";
const plain = value => JSON.parse(JSON.stringify(value));

// A stand-in for Cursor's data service. The unload method keeps the shape the guard checks for:
// the chat is read through the first parameter, then filtered by its generating bubbles.
function fixture({ surface = "agents", verifier = preflightSource, clock = { now: 1_000_000 } } = {}) {
  const natives = [];
  class Service {
    constructor() {
      this._environmentService = { isGlass: surface === "agents" };
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
    _composerDataIsBusy(data) { return data.applying === true; }
    unloadBubblesByIds(t, e, n) {
      natives.push({ self: this, count: arguments.length, handle: t, ids: e, extra: n });
      const a = this.getComposerData(t);
      if (!a) return;
      const generating = new Set(a.generatingBubbleIds ?? []);
      for (const id of e) if (!generating.has(id) && a.conversationMap[id] != null) delete a.conversationMap[id];
      return "native";
    }
  }
  const service = new Service();
  const prototype = Service.prototype;
  const context = vm.createContext({ __service: service, Date: { now: () => clock.now } });
  const evaluate = expression => vm.runInContext(`(function(){return ${expression};}).call(__service)`, context);
  const prepare = (owner = "owner", revision = "revision", lease = "lease", verify = verifier) => evaluate(
    guardSource.replace("__ASKHUMAN_OWNER__", () => JSON.stringify(owner))
      .replace("__ASKHUMAN_REVISION__", () => JSON.stringify(revision))
      .replace("__ASKHUMAN_LEASE__", () => JSON.stringify(lease))
      .replace("__ASKHUMAN_RETENTION_PREFLIGHT__", () => verify));
  const state = () => context[key];
  const install = async (...args) => { prepare(...args); return plain(await state().activate()); };
  // A chat with `count` bubbles whose human message is first and whose last bubble is the AI's.
  const chat = (id, { status = "completed", loaded = true, generating = [], count = 6, applying = false } = {}) => {
    const headers = Array.from({ length: count }, (_, index) => ({ bubbleId: `${id}-${index}`, type: index === 0 ? "human" : "ai" }));
    const conversationMap = {};
    for (const header of headers) if (loaded) conversationMap[header.bubbleId] = { bubbleId: header.bubbleId };
    const handle = { composerId: id, data: { status, headers, conversationMap, generatingBubbleIds: generating, applying } };
    service.handles.set(id, handle);
    service.loadedComposers.byId[id] = handle.data;
    return handle;
  };
  const ids = handle => handle.data.headers.map(header => header.bubbleId);
  return { service, prototype, natives, context, clock, evaluate, prepare, install, state, chat, ids };
}

test("a running chat keeps every loaded message while other chats unload natively", async () => {
  const f = fixture(), original = f.prototype[method];
  assert.equal((await f.install()).state, "active");
  const running = f.chat("running", { status: "generating" });
  assert.equal(f.service[method](running, f.ids(running)), undefined);
  assert.equal(Object.keys(running.data.conversationMap).length, 6);
  assert.equal(f.natives.length, 0);

  const idle = f.chat("idle");
  assert.equal(f.service[method](idle, f.ids(idle).slice(1, 4)), "native");
  assert.equal(Object.keys(idle.data.conversationMap).length, 3);
  assert.equal(f.natives.length, 1);
  assert.notEqual(f.prototype[method], original);
});

test("Cursor's own busy notions are all honored", async () => {
  const f = fixture();
  await f.install();
  for (const options of [{ status: "generating" }, { generating: ["live"] }, { applying: true }]) {
    const handle = f.chat(`busy-${JSON.stringify(options)}`, options);
    f.service[method](handle, f.ids(handle));
    assert.equal(Object.keys(handle.data.conversationMap).length, 6, JSON.stringify(options));
  }
  assert.equal(f.natives.length, 0);
});

test("the receiver, every argument, the return value, and errors are untouched for other chats", async () => {
  const f = fixture();
  await f.install();
  const idle = f.chat("idle");
  assert.equal(f.service[method](idle, ["idle-1"], "context", "future-argument"), "native");
  assert.equal(f.natives[0].self, f.service);
  assert.equal(f.natives[0].count, 4);
  assert.equal(f.natives[0].handle, idle);
  assert.deepEqual(f.natives[0].ids, ["idle-1"]);
  // The method has had three and four parameters across Cursor releases; two arguments work too.
  f.service[method](idle, ["idle-2"]);
  assert.equal(f.natives[1].count, 2);
});

test("a failure inside Cursor's own method is not hidden", async () => {
  const f = fixture();
  await f.install();
  const boom = new Error("native unload failed");
  const failing = { [Symbol.iterator]() { throw boom; } };
  assert.throws(() => f.service[method](f.chat("idle"), failing), error => error === boom);
});

test("an unreadable chat is never blocked and the guard itself never throws", async () => {
  const f = fixture();
  await f.install();
  const broken = { get data() { throw new Error("unreadable"); } };
  assert.equal(f.service[method](broken, ["a"]), undefined);
  assert.equal(f.natives.length, 1, "the native method still ran for a chat that could not be read");
  assert.equal(f.service[method](undefined, ["a"]), undefined);
  assert.equal(f.natives.length, 2);
  f.service._composerDataIsBusy = () => { throw new Error("busy check failed"); };
  const idle = f.chat("idle");
  assert.equal(f.service[method](idle, ["idle-1"]), "native");
});

test("skips are counted without identifiers or content and the recent list stays bounded", async () => {
  const f = fixture();
  await f.install();
  const running = f.chat("secret-chat-id", { status: "generating", count: 10 });
  for (let call = 0; call < 25; call++) {
    f.clock.now += 1000;
    f.service[method](running, call % 2 ? f.ids(running) : f.ids(running).slice(0, 3));
  }
  f.service[method](f.chat("idle"), []);
  const status = plain(f.state().status());
  assert.equal(status.skipped, 25);
  assert.equal(status.skippedBubbles, 12 * 10 + 13 * 3);
  assert.equal(status.passed, 1);
  assert.equal(status.lastSkippedAt, f.clock.now);
  assert.equal(status.recent.length, 20);
  assert.ok(status.recent.every(entry => Object.keys(entry).sort().join() === "at,bubbles,scope"));
  assert.deepEqual([...new Set(status.recent.map(entry => entry.scope))].sort(), ["all", "partial"]);
  assert.ok(!JSON.stringify(status).includes("secret-chat-id"));
});

test("only Agents windows are guarded; other windows are reported as not applicable and left alone", async () => {
  const f = fixture({ surface: "ide" }), original = f.prototype[method];
  const status = await f.install();
  assert.equal(status.state, "notApplicable");
  assert.equal(status.surface, "ide");
  assert.equal(f.prototype[method], original);
  assert.equal(f.state().restore(), true);
  assert.equal(f.state(), undefined);
});

test("attachment is idempotent and refuses a different owner, version, or installation", async () => {
  const f = fixture();
  await f.install();
  const own = f.prototype[method];
  assert.equal(plain(f.prepare()).state, "active");
  assert.equal(f.prototype[method], own);
  assert.throws(() => f.prepare("other"), /Another AskHuman/);
  assert.throws(() => f.prepare("owner", "next"), /different adapter version/);
  assert.throws(() => f.prepare("owner", "revision", "next"), /different adapter installation/);
  assert.equal(f.prototype[method], own);
});

test("restoring puts back the exact original property and a restored guard attaches again", async () => {
  const f = fixture();
  const before = Object.getOwnPropertyDescriptor(f.prototype, method);
  await f.install();
  const running = f.chat("running", { status: "generating" });
  assert.equal(f.state().restore(), true);
  assert.deepEqual(Object.getOwnPropertyDescriptor(f.prototype, method), before);
  assert.equal(f.state(), undefined);
  f.service[method](running, f.ids(running).slice(0, 2));
  assert.equal(Object.keys(running.data.conversationMap).length, 4, "native behavior is back");
  assert.equal((await f.install()).state, "active");
  f.state().restore();
});

test("a wrapper installed after ours is preserved and the retained guard defers to Cursor", async () => {
  const f = fixture();
  await f.install();
  const ours = f.prototype[method];
  const later = function (...args) { return ours.apply(this, args); };
  f.prototype[method] = later;
  const reported = plain(f.state().status());
  assert.equal(reported.state, "unavailable");
  assert.match(reported.error, /changed after attachment/);
  assert.equal(f.state().restore(), false);
  assert.equal(f.prototype[method], later);
  const running = f.chat("running", { status: "generating" });
  f.service[method](running, f.ids(running).slice(0, 2));
  assert.equal(Object.keys(running.data.conversationMap).length, 4, "a disabled retained guard must not skip");
});

for (const [name, mutate, message] of [
  ["a missing method", proto => { delete proto[method]; }, /method is unavailable/],
  ["a non-function", proto => { Object.defineProperty(proto, method, { value: 42, writable: true, configurable: true }); }, /method is unavailable/],
  ["a non-configurable method", proto => { Object.defineProperty(proto, method, { configurable: false }); }, /not replaceable/],
  ["a read-only method", proto => { Object.defineProperty(proto, method, { writable: false }); }, /not replaceable/],
  ["another first parameter use", proto => { proto[method] = { [method](t, e) { const a = this.getComposerData(e); return a?.generatingBubbleIds?.concat(a.conversationMap); } }[method]; }, /unexpected method shape/],
  ["no generating-bubble list", proto => { proto[method] = { [method](t, e) { const a = this.getComposerData(t); return a?.conversationMap; } }[method]; }, /unexpected method shape/],
  ["no message map", proto => { proto[method] = { [method](t, e) { const a = this.getComposerData(t); return a?.generatingBubbleIds; } }[method]; }, /unexpected method shape/],
  ["default parameters", proto => { proto[method] = { [method](t = 1, e) { const a = this.getComposerData(t); return [a?.generatingBubbleIds, a?.conversationMap]; } }[method]; }, /unexpected method shape/],
  ["a native function", proto => { proto[method] = Math.max; }, /unexpected method shape/],
]) {
  test(`a changed contract is reported unavailable without mutation: ${name}`, async () => {
    const f = fixture();
    mutate(f.prototype);
    const before = Object.getOwnPropertyDescriptor(f.prototype, method);
    const status = plain(f.prepare());
    assert.equal(status.state, "unavailable");
    assert.match(status.error, message);
    assert.deepEqual(Object.getOwnPropertyDescriptor(f.prototype, method), before);
    assert.equal(plain(await f.state().activate()).state, "unavailable");
    assert.deepEqual(Object.getOwnPropertyDescriptor(f.prototype, method), before);
    assert.equal(f.state().restore(), true);
  });
}

test("a failing or too-small preflight leaves Cursor untouched", async () => {
  for (const verifier of [
    "(() => { throw new Error('Retention preflight failed: injected'); })",
    "(() => ({ passed: false, checks: 10 }))",
    "(() => ({ passed: true, checks: 5 }))",
  ]) {
    const f = fixture({ verifier }), before = Object.getOwnPropertyDescriptor(f.prototype, method);
    const status = plain(f.prepare());
    assert.equal(status.state, "pending");
    const result = plain(await f.state().activate());
    assert.equal(result.state, "unavailable");
    assert.ok(result.error);
    assert.deepEqual(Object.getOwnPropertyDescriptor(f.prototype, method), before);
  }
});

test("a method replaced between preparation and activation is preserved", async () => {
  const f = fixture();
  f.prepare();
  // The replacement has an acceptable shape, so only the identity comparison can catch it.
  const replacement = { [method](t, e, n) { const a = this.getComposerData(t); return [a?.generatingBubbleIds, a?.conversationMap]; } }[method];
  f.prototype[method] = replacement;
  const status = plain(await f.state().activate());
  assert.equal(status.state, "unavailable");
  assert.match(status.error, /changed before activation/);
  assert.equal(f.prototype[method], replacement);
});

test("activation after restoration is refused", async () => {
  const f = fixture();
  f.prepare();
  const state = f.state();
  assert.equal(state.restore(), true);
  assert.throws(() => state.activate(), /cancelled/);
  assert.equal(f.state(), undefined);
});

test("the production wrapper passes the production preflight with enough checks", async () => {
  const f = fixture();
  const status = await f.install();
  assert.equal(status.state, "active");
  assert.ok(status.checks >= 6);
  assert.equal(status.skipped, 0, "preflight must use isolated counters");
  assert.equal(status.passed, 0);
});

test("the at-risk gauge counts running chats whose last user message cannot be reached", async () => {
  const f = fixture();
  await f.install();
  const reachable = f.chat("reachable", { status: "generating" });
  const gap = f.chat("gap", { status: "generating" });
  delete gap.data.conversationMap["gap-3"];
  f.chat("gone", { status: "generating", loaded: false });
  f.chat("idle-gap", {});
  delete f.service.handles.get("idle-gap").data.conversationMap["idle-gap-3"];
  // Activation already sampled once (with no chats); the next sample is allowed 30 seconds later.
  assert.equal(plain(f.state().status()).atRisk, 0);
  f.clock.now += 30_000;
  const first = plain(f.state().status());
  assert.equal(first.atRisk, 2, "gap and gone are at risk; an idle chat and a reachable one are not");
  assert.equal(first.atRiskAt, f.clock.now);
  // The count is cached for 30 seconds, then recomputed.
  delete reachable.data.conversationMap["reachable-5"];
  delete reachable.data.conversationMap["reachable-0"];
  f.clock.now += 29_000;
  assert.equal(plain(f.state().status()).atRisk, 2);
  f.clock.now += 1_000;
  assert.equal(plain(f.state().status()).atRisk, 3);
});

test("the at-risk gauge reports unknown when Cursor's helpers are missing or fail, without affecting the guard", async () => {
  const f = fixture();
  await f.install();
  delete f.prototype.getLoadedConversation;
  f.clock.now += 30_000;
  const unknown = plain(f.state().status());
  assert.equal(unknown.state, "active");
  assert.equal(unknown.atRisk, null);

  const g = fixture();
  await g.install();
  g.chat("running", { status: "generating" });
  g.service.getLastHumanBubbleId = () => { throw new Error("helper failed"); };
  g.clock.now += 30_000;
  const failed = plain(g.state().status());
  assert.equal(failed.state, "active");
  assert.equal(failed.atRisk, null);
  const running = g.service.handles.get("running");
  g.service[method](running, g.ids(running));
  assert.equal(Object.keys(running.data.conversationMap).length, 6);
});

test("the gauge examines a bounded number of running chats", async () => {
  const f = fixture();
  await f.install();
  for (let index = 0; index < 80; index++) {
    const handle = f.chat(`running-${index}`, { status: "generating" });
    delete handle.data.conversationMap[`running-${index}-5`];
    delete handle.data.conversationMap[`running-${index}-0`];
  }
  f.clock.now += 30_000;
  assert.equal(plain(f.state().status()).atRisk, 50);
});

// --- The preflight must catch a wrapper that misbehaves (differential check) -----------------

const verifier = vm.runInNewContext(preflightSource);

function reference(fault = "") {
  return original => {
    const control = { enabled: true, skipped: 0, skippedBubbles: 0, passed: 0, lastSkippedAt: 0, recent: [] };
    const running = (self, handle) => {
      try {
        const chat = self.getComposerData(handle);
        if (!chat) return undefined;
        if (chat.status === "generating" && fault !== "ignores-status") return chat;
        if (chat.generatingBubbleIds?.length > 0 && fault !== "ignores-live-bubbles") return chat;
        if (self._composerDataIsBusy?.(chat) === true && fault !== "ignores-native-busy") return chat;
      } catch { if (fault === "blocks-unreadable") return {}; }
      return undefined;
    };
    const wrapper = function (...args) {
      if (control.enabled || fault === "ignores-disabled") {
        const chat = fault === "never-skips" ? undefined : running(this, args[0]);
        if (chat || fault === "always-skips") {
          control.skipped++; control.skippedBubbles += Array.isArray(args[1]) ? args[1].length : 0;
          control.lastSkippedAt = Date.now();
          const loaded = Object.keys(chat?.conversationMap ?? {}).length;
          const entry = { at: control.lastSkippedAt, bubbles: args[1]?.length ?? 0, scope: loaded > 0 && (args[1]?.length ?? 0) >= loaded ? "all" : "partial" };
          if (fault === "leaks-identity") entry.composerId = args[0]?.composerId ?? "id";
          control.recent.push(entry);
          if (fault !== "unbounded-recent" && control.recent.length > 20) control.recent.shift();
          return undefined;
        }
      }
      if (fault !== "uncounted-pass") control.passed++;
      if (fault === "swallows-errors") { try { return original.apply(this, args); } catch { return undefined; } }
      if (fault === "wrong-receiver") return original.apply(undefined, args);
      if (fault === "drops-arguments") return original.apply(this, args.slice(0, 2));
      if (fault === "calls-twice") { original.apply(this, args); return original.apply(this, args); }
      if (fault === "wrong-result") { original.apply(this, args); return "other"; }
      return original.apply(this, args);
    };
    return { wrapper, control };
  };
}

test("the preflight accepts a correct wrapper", () => {
  const result = verifier({ create: reference() });
  assert.equal(result.passed, true);
  assert.ok(result.checks >= 6);
});

for (const fault of [
  "ignores-status", "ignores-live-bubbles", "ignores-native-busy", "never-skips", "always-skips", "blocks-unreadable",
  "ignores-disabled", "swallows-errors", "wrong-receiver", "drops-arguments", "calls-twice", "wrong-result",
  "uncounted-pass", "unbounded-recent", "leaks-identity",
]) {
  test(`the preflight rejects a wrapper that ${fault.replace(/-/g, " ")}`, () => {
    assert.throws(() => verifier({ create: reference(fault) }), /Retention preflight failed/);
  });
}
