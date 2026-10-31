(() => {
  "use strict";
  const key = "__ASKHUMAN_CURSOR_MESSAGES_V1__";
  const owner = __ASKHUMAN_OWNER__, revision = __ASKHUMAN_REVISION__, lease = __ASKHUMAN_LEASE__;
  const existing = globalThis[key];
  if (existing) {
    if (existing.owner !== owner || existing.revision !== revision || existing.lease !== lease) throw new Error("Message adapter identity differs");
    return existing.status();
  }
  const data = this;
  let chat;
  for (let scope = data?._instantiationService; scope; scope = scope._parent) {
    for (const [id, service] of scope._services?._entries ?? []) if (String(id) === "composerChatService") chat ??= service;
  }
  if (!chat || typeof data.getHandleIfLoaded !== "function" ||
      !["acquirePreGenerationSubmission", "isQuietForBackgroundSubmit", "_submitChatMaybeAbortCurrent"].every(k => typeof chat[k] === "function")) {
    throw new Error("Unsupported native message contract");
  }
  let enabled = true;
  const operations = new Map();
  const state = {
    owner, revision, lease,
    status: () => ({ prepared: enabled, enabled, intact: enabled, verified: enabled, checks: 1, surface: "messages" }),
    activate() { return this.status(); },
    owns(id) { return enabled && !!data.getHandleIfLoaded(id); },
    restore() { enabled = false; if (globalThis[key] === state) delete globalThis[key]; return true; },
    send(operation) {
      if (!enabled || !operation || typeof operation.id !== "string" || typeof operation.sessionId !== "string" || typeof operation.text !== "string") {
        return Promise.resolve({ status: "rejected", reason: "nativeUnavailable" });
      }
      const body = JSON.stringify(operation);
      const previous = operations.get(operation.id);
      if (previous) return previous.body === body ? previous.promise : Promise.resolve({ status: "rejected", reason: "conflict" });
      // Keep the same promise even if the window's caller loses its acknowledgment.
      const promise = (async () => {
        let release, submitted = false, timer;
        try {
          const handle = data.getHandleIfLoaded(operation.sessionId);
          if (!handle) return { status: "rejected", reason: "notFound" };
          release = await chat.acquirePreGenerationSubmission(operation.sessionId);
          if (typeof release !== "function") return { status: "rejected", reason: "nativeUnavailable" };
          const unlock = () => { const fn = release; release = undefined; fn?.(); };
          if (!enabled || !chat.isQuietForBackgroundSubmit(handle)) return { status: "rejected", reason: "stateChanged" };
          return await new Promise(resolve => {
            let settled = false;
            const finish = result => { if (!settled) { settled = true; clearTimeout(timer); resolve(result); } };
            timer = setTimeout(() => finish({ status: "unknown", reason: "controlUnknown" }), 2500);
            // The native submission lock excludes a competing send; the native quiet predicate
            // rejects running turns, pending plans, user decisions, and queued messages.
            submitted = true;
            Promise.resolve(chat._submitChatMaybeAbortCurrent(operation.sessionId, operation.text, {
              forceBubbleId: operation.id, ignoreQueuing: true, skipClearInput: true,
              skipFocusAfterSubmission: true, _internalPreserveDraft: true,
              _internalOnGenerationClaimed: () => {
                const current = data.getHandleIfLoaded(operation.sessionId)?.data;
                const turnId = current?.chatGenerationUUID ?? current?.conversationActionManager?.getGenerationUUID?.();
                unlock();
                finish(typeof turnId === "string" && turnId ? { status: "started", turnId } : { status: "unknown", reason: "controlUnknown" });
              },
            })).then(() => finish({ status: "unknown", reason: "controlUnknown" }), () => finish({ status: "unknown", reason: "controlUnknown" }))
              .finally(unlock);
          });
        } catch {
          clearTimeout(timer);
          release?.();
          return { status: submitted ? "unknown" : "rejected", reason: submitted ? "controlUnknown" : "nativeUnavailable" };
        } finally {
          // A submitted promise owns the lock until its claim callback or native completion.
          if (!submitted) release?.();
        }
      })();
      operations.set(operation.id, { body, promise });
      return promise;
    },
  };
  globalThis[key] = state;
  return state.status();
})()
