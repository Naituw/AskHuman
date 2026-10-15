import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";
import { test } from "node:test";

const source = fs.readFileSync(new URL("../src-tauri/src/cursor_desktop/preflight.js", import.meta.url), "utf8");
const verify = vm.runInNewContext(source);

function fixture(fault = "", paired = true) {
  const disposal = { stores: 0, services: 0, listeners: 0 };
  let unblock;
  class Store {
    constructor() { this.pending = new Map(); this.wakeups = { inFlightById: new Map() }; this.listeners = new Set(); }
    onDidSettleCompletion(fn) { this.listeners.add(fn); return { dispose: () => { this.listeners.delete(fn); disposal.listeners++; } }; }
    settle(item) { for (const fn of this.listeners) fn({ conversationId: item.conversationId, taskId: item.id }); }
    dispose() { disposal.stores++; }
  }
  class Background {
    constructor(store) { this.store = store; }
    enqueueCompletion(payload) { this.store.pending.set(payload.taskId, { id: payload.taskId, conversationId: payload.composerId, payload }); }
    pull() {
      const items = [...this.store.pending.values()]; this.store.pending.clear();
      for (const item of items) this.store.wakeups.inFlightById.set(item.id, item);
      return items;
    }
    ack(ids) {
      if (fault === "async-ack") return Promise.resolve();
      if (fault === "lost-ack") return;
      for (const id of ids) {
        const item = this.store.wakeups.inFlightById.get(id);
        if (!item) continue;
        if (fault !== "stale-inflight") this.store.wakeups.inFlightById.delete(id);
        this.store.settle(item);
        if (fault === "repeat-ack") this.store.pending.set(id, item);
      }
    }
    nack(ids, { requeue }) {
      for (const id of ids) {
        const item = this.store.wakeups.inFlightById.get(id);
        this.store.wakeups.inFlightById.delete(id);
        if (item && requeue) this.store.pending.set(id, item);
      }
    }
    hasPendingCompletions() { return this.store.pending.size > 0; }
    markAwaitedCompletion(_id, task) {
      const item = this.store.pending.get(task); this.store.pending.delete(task);
      if (item) this.store.settle(item);
    }
    unmarkAwaitedCompletion() {}
    dispose() { disposal.services++; }
  }
  class Agent {
    async _maybeDispatchBackgroundCompletions(id) {
      if (fault === "stall") return new Promise(resolve => { unblock = resolve; });
      const handle = this._composerDataService.getHandleIfLoaded(id);
      if (handle.data.chatGenerationUUID) return;
      if (fault === "model") return this._agentService.runAgentLoop();
      const bg = this._backgroundWorkService;
      this._completionDispatchInFlightComposerIds.add(id);
      try {
        while (bg.hasPendingCompletions(id)) {
          const batch = bg.pull(id), awaited = this._pendingAwaitTaskIdsByComposerId.get(id) ?? new Set();
          const ready = batch.filter(item => fault === "ignore-await" || !awaited.has(item.payload.taskId));
          bg.nack(batch.filter(item => !ready.includes(item)).map(item => item.id), { requeue: true });
          if (!ready.length) return;
          if (await this._runBackgroundTaskCompletionAction() === "completed") {
            bg.ack(ready.map(item => item.id));
            if (paired) await this._commitPairedCloudChildInboxCursor(handle, ready.map(item => item.payload.outputPath).filter(Boolean).sort());
          }
        }
      } finally { this._completionDispatchInFlightComposerIds.delete(id); }
    }
    async _commitPairedCloudChildInboxCursor(handle, paths) {
      if (!paths.length) return;
      const files = (await this._fileService.resolve()).children.map(item => "inbox/subscription-results/" + item.name);
      const last = paths.at(-1), previous = handle.data.interactiveChildInboxLastPresentedKey;
      if (fault === "skip-gap" || !files.some(path => (!previous || path > previous) && path < last && !paths.includes(path))) {
        this._composerDataService.updateComposerDataSetStore(handle, set => set("interactiveChildInboxLastPresentedKey", last));
      }
    }
  }
  if (!paired) delete Agent.prototype._commitPairedCloudChildInboxCursor;
  const live = new Store(), background = new Background(live);
  background.enqueueCompletion({ composerId: "real-chat", taskId: "live-task", status: "error" });
  return { live, disposal, unblock: () => unblock(), run: (options = {}) => verify({ prototype: Agent.prototype, background,
    handler: async () => "completed", id: "isolated-probe", cancelled: () => false, ...options }) };
}

for (const paired of [false, true]) test(`preflight checks native delivery and leaves live queue untouched (paired=${paired})`, async () => {
  const f = fixture("", paired), result = await f.run();
  assert.equal(result.passed, true);
  assert.equal(result.checks, paired ? 8 : 6);
  assert.equal(f.live.pending.size, 1);
  assert.equal(f.live.pending.get("live-task").payload.status, "error");
  assert.equal(f.live.wakeups.inFlightById.size, 0);
  assert.deepEqual(f.disposal, { stores: 1, services: 1, listeners: 1 });
});

for (const [fault, expected] of [
  ["async-ack", /must settle synchronously/], ["lost-ack", /batch outcomes/],
  ["stale-inflight", /batch outcomes/], ["ignore-await", /Await exclusion/],
  ["model", /model boundary/], ["skip-gap", /gap preservation/], ["repeat-ack", /budget/],
]) test(`preflight rejects ${fault} and disposes isolated resources`, async () => {
  const f = fixture(fault);
  await assert.rejects(f.run(), expected);
  assert.equal(f.live.pending.size, 1);
  assert.deepEqual(f.disposal, { stores: 1, services: 1, listeners: 1 });
});

test("cancellation disposes resources even before a stalled native call returns", async () => {
  const f = fixture("stall");
  let cancelled = false, cleanup;
  const pending = f.run({ cancelled: () => cancelled, onCleanup: fn => { cleanup = fn; } });
  cancelled = true; cleanup();
  assert.deepEqual(f.disposal, { stores: 1, services: 1, listeners: 1 });
  f.unblock();
  await assert.rejects(pending, /cancelled/);
  assert.deepEqual(f.disposal, { stores: 1, services: 1, listeners: 1 });
});
