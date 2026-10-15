(async function verifyNativeCompletion(context) {
  "use strict";
  const { prototype, background, handler, id, cancelled } = context;
  const deadline = Date.now() + 1000;
  let operations = 0, checks = 0, store, service, subscription;
  let disposed = false;
  const cleanup = () => {
    if (disposed) return;
    disposed = true;
    try { subscription?.dispose(); }
    finally { try { service?.dispose(); } finally { store?.dispose(); } }
  };
  context.onCleanup?.(cleanup);
  const tick = () => {
    if (cancelled() || Date.now() > deadline || ++operations > 160) {
      throw new Error("Native completion preflight exceeded its budget or was cancelled");
    }
  };
  const check = (condition, name) => {
    tick();
    if (!condition) throw new Error(`Native completion preflight failed: ${name}`);
    checks++;
  };
  const forbidden = () => { throw new Error("Native preflight reached an unexpected model boundary"); };
  try {
    // These fresh native objects have no live conversations, listeners, or resource providers.
    store = new background.store.constructor();
    service = new (Object.getPrototypeOf(background).constructor)(store);
    check(service.store === store && store !== background.store, "isolated queue ownership");
    const queue = {};
    for (const method of ["enqueueCompletion", "pull", "ack", "nack", "hasPendingCompletions",
      "markAwaitedCompletion", "unmarkAwaitedCompletion"]) {
      if (typeof service[method] !== "function") throw new Error(`Missing native queue method: ${method}`);
      queue[method] = (...args) => {
        tick();
        const value = service[method](...args);
        if (value && typeof value.then === "function") {
          Promise.resolve(value).catch(() => {});
          throw new Error(`Native queue ${method} must settle synchronously`);
        }
        return value;
      };
    }
    const settled = new Set();
    subscription = store.onDidSettleCompletion(event => {
      if (event.conversationId !== id) throw new Error("Unexpected preflight conversation");
      settled.add(event.taskId);
    });
    const handle = { composerId: id, data: { composerId: id, isNAL: true, status: "none" } };
    const data = {
      getHandleIfLoaded: key => key === id ? handle : undefined,
      getComposerData: value => value.data,
      getComposerDataIfLoaded: key => key === id ? handle.data : undefined,
      getIsBlockingUserDecision: () => false,
      getToolFormer: () => undefined,
      updateComposerDataSetStore: (value, update) => update((key, next) => { value.data[key] = next; }),
      appendComposerBubbles: () => { throw new Error("Preflight attempted to append a chat message"); },
    };
    let files = [], handled = 0;
    const agent = Object.assign(Object.create(prototype), {
      _backgroundWorkService: queue,
      _composerDataService: data,
      _experimentService: { checkFeatureGate: () => true },
      _composerUtilsService: { getHasPendingQuestionnaireAwaitingInput: () => false },
      _completionDispatchInFlightComposerIds: new Set(),
      _pendingAwaitTaskIdsByComposerId: new Map(),
      _ownsBackgroundCompletion: () => true,
      _aiService: { streamingAbortControllers: new Map([["probe-turn", {}]]), getModelDetails: forbidden },
      _agentService: { runAgentLoop: forbidden },
      _runWakeupAction: forbidden,
      _runBackgroundTaskCompletionAction(...args) { tick(); handled++; return handler.apply(this, args); },
      _structuredLogService: {
        info() {},
        warn() { throw new Error("Native preflight encountered an inbox error"); },
        error(_area, _message, error) { throw error; },
      },
      runIdleHooks() {},
      _commandService: { executeCommand: async () => {
        tick(); return { scopes: [{ scope: "agent", path: "/askhuman-preflight-virtual-store" }] };
      } },
      _fileService: { resolve: async () => {
        tick(); return { children: files.map(path => ({ name: path.split("/").at(-1), isDirectory: false })) };
      } },
    });
    const enqueue = (taskId, kind = "shell", status = "success", outputPath) =>
      queue.enqueueCompletion({ composerId: id, taskId, kind, status, outputPath });
    const dispatch = async () => { tick(); await prototype._maybeDispatchBackgroundCompletions.call(agent, id); tick(); };
    const empty = () => !queue.hasPendingCompletions(id) &&
      store.wakeups?.inFlightById?.size === 0 && agent._completionDispatchInFlightComposerIds.size === 0;

    for (const kind of ["shell", "subagent"]) {
      for (const status of ["success", "error", "aborted"]) enqueue(`${kind}-${status}`, kind, status);
    }
    await dispatch();
    check(handled === 1 && settled.size === 6 && empty(), "batch outcomes and synchronous ACK");
    agent._pendingAwaitTaskIdsByComposerId.set(id, new Set(["awaited"]));
    enqueue("awaited"); enqueue("other"); await dispatch();
    check(queue.hasPendingCompletions(id) && settled.has("other") && !settled.has("awaited"), "Await exclusion");
    queue.markAwaitedCompletion(id, "awaited");
    check(settled.has("awaited") && empty(), "Await native settlement");
    agent._pendingAwaitTaskIdsByComposerId.delete(id);
    queue.unmarkAwaitedCompletion(id, "awaited");
    handle.data.chatGenerationUUID = "probe-turn";
    enqueue("busy"); await dispatch();
    check(queue.hasPendingCompletions(id) && !settled.has("busy"), "active turn queue retention");
    delete handle.data.chatGenerationUUID;
    handle.data.status = "aborted"; handle.data.abortReason = "user";
    await dispatch();
    check(settled.has("busy") && empty(), "completion after Stop");
    if (typeof prototype._dropUserCancelledSubagentWakeups === "function") {
      const before = handled;
      enqueue("cancelled-child", "subagent", "aborted"); await dispatch();
      check(empty() && handled === before, "native cancelled child exclusion");
      enqueue("successful-child", "subagent"); await dispatch();
      check(empty() && handled === before + 1 && settled.has("successful-child"), "successful child after Stop");
    }
    handle.data.status = "completed"; delete handle.data.abortReason;
    const paired = typeof prototype._commitPairedCloudChildInboxCursor === "function";
    if (paired) {
      const path = n => `inbox/subscription-results/2026-01-01T00-00-0${n}-000Z_child-turn_11111111-1111-4111-8111-111111111111_22222222-2222-4222-8222-222222222222_run${n}.jsonl`;
      files = [path(1), path(2)];
      for (const n of [2, 1]) enqueue(`paired-${n}`, "subagent", "success", path(n));
      await dispatch();
      check(empty() && settled.has("paired-1") && settled.has("paired-2") &&
        handle.data.interactiveChildInboxLastPresentedKey === path(2), "paired inbox sorted cursor");
      files.push(path(3), path(4));
      enqueue("paired-gap", "subagent", "success", path(4)); await dispatch();
      check(empty() && settled.has("paired-gap") &&
        handle.data.interactiveChildInboxLastPresentedKey === path(2), "paired inbox gap preservation");
    }
    return { passed: true, checks, profile: paired ? "paired-inbox" : "direct-ack" };
  } finally {
    cleanup();
  }
})
