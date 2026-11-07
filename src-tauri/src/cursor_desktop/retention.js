(() => {
  "use strict";
  // Running-chat protection for Cursor's Agents window. A chat that is still generating keeps its
  // loaded messages: Cursor's own unload entry point ignores it. See
  // docs/specs/cursor-desktop-background-followup.md section 7.
  const key = "__ASKHUMAN_CURSOR_RETENTION_V1__";
  const owner = __ASKHUMAN_OWNER__;
  const revision = __ASKHUMAN_REVISION__;
  const lease = __ASKHUMAN_LEASE__;
  const verify = __ASKHUMAN_RETENTION_PREFLIGHT__;
  const method = "unloadBubblesByIds";
  const existing = globalThis[key];
  if (existing) {
    if (existing.owner !== owner) throw new Error("Another AskHuman instance owns this window");
    if (existing.revision !== revision) throw new Error("A different adapter version requires restoration");
    if (existing.lease !== lease) throw new Error("A different adapter installation requires restoration");
    return existing.status();
  }
  const service = this;
  const surface = service?._environmentService?.isGlass === true ? "agents" : "ide";

  // The chat's data when Cursor itself would consider it busy, otherwise undefined. Never throws:
  // a record that cannot be read is treated as idle so Cursor's own behavior is left alone.
  function runningChat(target, handle) {
    let chat;
    try { chat = target.getComposerData(handle); } catch { return undefined; }
    if (!chat) return undefined;
    try {
      if (chat.status === "generating" || chat.generatingBubbleIds?.length > 0) return chat;
    } catch { /* A malformed record is idle. */ }
    try { if (target._composerDataIsBusy?.(chat) === true) return chat; } catch { /* Same. */ }
    return undefined;
  }

  function makeControl(enabled) {
    const control = {
      enabled, skipped: 0, skippedBubbles: 0, passed: 0, lastSkippedAt: 0, recent: [],
      skip(ids, chat) {
        // Observability is best effort and must never make an unload request fail.
        try {
          const bubbles = Array.isArray(ids) ? ids.length : 0;
          let loaded = 0;
          try { loaded = Object.keys(chat?.conversationMap ?? {}).length; } catch { /* Unknown. */ }
          control.skipped++;
          control.skippedBubbles += bubbles;
          control.lastSkippedAt = Date.now();
          control.recent.push({ at: control.lastSkippedAt, bubbles, scope: loaded > 0 && bubbles >= loaded ? "all" : "partial" });
          if (control.recent.length > 20) control.recent.shift();
        } catch { /* Ignore. */ }
      },
    };
    return control;
  }

  // Rest arguments: the method's arity has changed between Cursor releases, only the first
  // argument (the chat handle) is relied on.
  function makeWrapper(original, control) {
    return function (...args) {
      if (control.enabled) {
        const chat = runningChat(this, args[0]);
        if (chat) {
          control.skip(args[1], chat);
          return undefined;
        }
      }
      control.passed++;
      return original.apply(this, args);
    };
  }

  // Finds Cursor's method and checks it still has the shape this guard was written against:
  // replaceable, reads the chat through its first parameter, and works on the generating-bubble
  // list and the loaded-message map. Anything else is reported unavailable, never patched.
  function inspect(target) {
    let prototype, descriptor;
    for (prototype = Object.getPrototypeOf(target); prototype && prototype !== Object.prototype;
         prototype = Object.getPrototypeOf(prototype)) {
      descriptor = Object.getOwnPropertyDescriptor(prototype, method);
      if (descriptor) break;
    }
    const native = descriptor?.value;
    const unsupported = reason => new Error(`Unsupported conversation unload contract: ${reason}`);
    if (typeof native !== "function") throw unsupported("method is unavailable");
    if (descriptor.configurable !== true || descriptor.writable !== true) throw unsupported("method is not replaceable");
    if (typeof target.getComposerData !== "function") throw unsupported("chat reader is unavailable");
    const source = Function.prototype.toString.call(native);
    const first = /^[^(]*\(([^)]*)\)/.exec(source)?.[1].split(",")[0].trim() ?? "";
    const read = /\bthis\.getComposerData\(\s*([^()]*?)\s*\)/.exec(source)?.[1];
    if (!/^[\w$]+$/.test(first) || read !== first || !source.includes("generatingBubbleIds") ||
        !source.includes("conversationMap")) throw unsupported("unexpected method shape");
    return { prototype, descriptor };
  }

  // Running chats whose last user message can no longer be reached from the loaded tail would be
  // mistaken for an empty submit if they failed now. Diagnostic only: bounded, best effort, and
  // never able to affect the guard.
  function measureAtRisk() {
    const byId = service.loadedComposers?.byId;
    if (!byId || typeof service.getHandleIfLoaded !== "function" ||
        typeof service.getLastHumanBubbleId !== "function" ||
        typeof service.getLoadedConversation !== "function") return null;
    let inspected = 0, atRisk = 0;
    for (const id of Object.keys(byId).slice(0, 1000)) {
      const handle = service.getHandleIfLoaded(id);
      if (!handle || !runningChat(service, handle)) continue;
      if (++inspected > 50) break;
      const last = service.getLastHumanBubbleId(handle);
      if (last === undefined) continue;
      if (!service.getLoadedConversation(handle).some(bubble => bubble?.bubbleId === last)) atRisk++;
    }
    return atRisk;
  }

  let phase = surface === "agents" ? "pending" : "notApplicable";
  let failure, checks = 0, installed = false, cancelled = false;
  let prototype, descriptor, wrapper;
  const control = makeControl(false);
  const gauge = { at: 0, value: null };
  const fail = error => { phase = "unavailable"; failure = String(error?.message ?? error).slice(0, 200); };
  if (phase === "pending") {
    try { ({ prototype, descriptor } = inspect(service)); } catch (error) { fail(error); }
  }

  const state = {
    owner, revision, lease, surface,
    intact: () => installed && Object.getOwnPropertyDescriptor(prototype, method)?.value === wrapper,
    status() {
      const intact = installed ? this.intact() : false;
      // Someone replaced our wrapper after attachment: the chain can no longer be verified.
      const drifted = phase === "active" && !intact;
      if (phase === "active" && !drifted && Date.now() - gauge.at >= 30000) {
        gauge.at = Date.now();
        try { gauge.value = measureAtRisk(); } catch { gauge.value = null; }
      }
      return {
        prepared: true, state: drifted ? "unavailable" : phase, surface, intact, checks,
        error: drifted ? "The conversation unload handler changed after attachment" : failure,
        skipped: control.skipped, skippedBubbles: control.skippedBubbles, passed: control.passed,
        lastSkippedAt: control.lastSkippedAt, recent: control.recent.slice(),
        atRisk: gauge.value, atRiskAt: gauge.at,
      };
    },
    activate() {
      if (cancelled) throw new Error("Retention activation was cancelled");
      if (phase !== "pending" || installed) return this.status();
      try {
        // The window may have changed since the provider breakpoint, so read the contract again.
        const current = inspect(service);
        if (current.prototype !== prototype || current.descriptor.value !== descriptor.value) {
          throw new Error("Unsupported conversation unload contract: method changed before activation");
        }
        const verification = verify({
          create: original => {
            const isolated = makeControl(true);
            return { wrapper: makeWrapper(original, isolated), control: isolated };
          },
        });
        if (!verification?.passed || verification.checks < 6) throw new Error("Retention preflight did not pass");
        checks = verification.checks;
        wrapper = makeWrapper(descriptor.value, control);
        Object.defineProperty(prototype, method, { ...descriptor, value: wrapper });
        installed = true;
        if (Object.getOwnPropertyDescriptor(prototype, method)?.value !== wrapper) {
          Object.defineProperty(prototype, method, descriptor);
          installed = false;
          throw new Error("The conversation unload handler could not be replaced");
        }
        control.enabled = true;
        phase = "active";
      } catch (error) { fail(error); }
      return this.status();
    },
    restore() {
      cancelled = true;
      control.enabled = false;
      if (installed) {
        if (!this.intact()) return false;
        Object.defineProperty(prototype, method, descriptor);
        installed = false;
      }
      if (globalThis[key] === state) delete globalThis[key];
      return true;
    },
  };
  globalThis[key] = state;
  return state.status();
})()
