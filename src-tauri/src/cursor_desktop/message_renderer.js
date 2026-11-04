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

  // ---- Creating a task in the Agents window (docs/specs/cursor-desktop-task-creation.md) ----
  // A chat is created through the Agents window's own `agentRepositoryService.createAgent`, with a
  // pre-assigned composer id. Cursor calls `beforeInitialSubmit` after the chat exists and before
  // anything is sent to a model, and abandons the creation (discarding the chat) when it throws.
  // Every readback check, and the dry run, happens there, so a task whose chat is not exactly what
  // was asked for never reaches a model. Services are looked up when a task is created, not when
  // the adapter installs: the same adapter also serves classic IDE windows, which have none of them.
  const creations = new Map();
  const holds = new Set();
  const WAIT_MS = 2000, HOLD_MS = 10 * 60 * 1000, TIDY_MS = 1500, DEFAULT_WINDOW_MS = 45000;
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
  const isAgents = () => enabled && data._environmentService?.isGlass === true;
  const brief = value => String(value?.message ?? value ?? "").split("\n")[0].slice(0, 160);
  const failure = (reason, detail, extra) => ({
    status: "rejected", reason, ...(detail ? { detail: String(detail).slice(0, 200) } : {}), ...(extra ?? {}),
  });
  const dryRun = new Error("AskHuman dry run");
  const refusal = (reason, detail) => Object.assign(new Error(`AskHuman refused: ${reason}`), { askhuman: { reason, detail } });
  const plain = value => (value === undefined ? undefined : JSON.parse(JSON.stringify(value)));
  const samePairs = (left, right) => {
    const key = pair => `${pair?.id}=${pair?.value}`;
    const a = (left ?? []).map(key).sort(), b = (right ?? []).map(key).sort();
    return a.length === b.length && a.every((item, index) => item === b[index]);
  };

  function services() {
    const found = {};
    for (let scope = data?._instantiationService; scope; scope = scope._parent) {
      for (const [id, service] of scope._services?._entries ?? []) {
        const name = String(id);
        if (name === "agentRepositoryService") found.repo ??= service;
        else if (name === "workspaceMetadataService") found.metadata ??= service;
        else if (name === "modelConfigService") found.models ??= service;
      }
    }
    return found;
  }

  // The structural preflight: every service and method the creation touches, or nothing happens.
  function contract() {
    if (!isAgents()) return { reason: "notAgentsWindow" };
    const found = services();
    const callable = (service, names) => !!service && names.every(name => typeof service[name] === "function");
    if (!callable(found.repo, ["createAgent"])) return { reason: "nativeUnavailable", detail: "agent repository" };
    if (!callable(found.metadata, ["getAllMetadata"])) return { reason: "nativeUnavailable", detail: "workspace metadata" };
    if (!callable(found.models, ["getModelConfig"])) return { reason: "nativeUnavailable", detail: "model configuration" };
    return found;
  }

  const normalizePath = value => (typeof value === "string" && value.startsWith("/") ? (value.length > 1 ? value.replace(/\/+$/, "") : value) : undefined);

  // The project is one the Agents window already knows: its identifier comes from the window's own
  // workspace records, never from a path this adapter would have to turn into an identifier.
  function findProject(metadata, wanted) {
    const paths = new Set(wanted.map(normalizePath).filter(Boolean));
    let records;
    try { records = [...(metadata.getAllMetadata() ?? [])]; } catch (error) { return { error: "nativeUnavailable", detail: brief(error) }; }
    const found = new Map();
    for (const record of records) {
      if (!record || record.configPath || !Array.isArray(record.paths) || record.paths.length !== 1) continue;
      const uri = record.paths[0]?.uri;
      if (!uri || uri.scheme !== "file") continue;
      const path = normalizePath(uri.fsPath ?? uri.path);
      if (!path || !paths.has(path)) continue;
      let canonical;
      try { canonical = metadata.findCanonicalIdByFolderUri?.(uri); } catch { canonical = undefined; }
      const id = typeof canonical === "string" && canonical ? canonical : record.workspaceId;
      if (typeof id === "string" && id) found.set(id, { id, uri });
    }
    if (found.size === 1) return [...found.values()][0];
    return { error: found.size ? "projectAmbiguous" : "projectNotFound", detail: `${records.length} known projects` };
  }

  // The configuration the chat starts with: Cursor's own live default for new chats, with only the
  // model and the Max flag replaced when this task asks for them.
  function buildModelConfig(models, operation) {
    let base;
    try { base = models.getModelConfig("composer"); } catch (error) { return { error: "modelUnavailable", detail: brief(error) }; }
    if (!base || typeof base !== "object") return { error: "modelUnavailable" };
    const next = plain(base);
    const pick = operation.model;
    if (pick) {
      if (typeof pick.modelName !== "string" || !pick.modelName || !Array.isArray(pick.parameters)) return { error: "modelUnavailable" };
      next.modelName = pick.modelName;
      next.selectedModels = [{ modelId: pick.modelName, parameters: pick.parameters.map(item => ({ id: String(item.id), value: String(item.value) })) }];
    }
    if (typeof operation.maxMode === "boolean") next.maxMode = operation.maxMode;
    let config = next;
    try { config = models.fixupModelConfigForCurrentFlag?.(next) ?? next; } catch { config = next; }
    return { config };
  }

  // Compares what Cursor made with what was asked for; returns the reason of the first difference.
  function verify(reference, operation, project, config) {
    const id = reference?.header?.id;
    if (id !== operation.id) return { reason: "composerMismatch", detail: "chat id" };
    const composer = (reference.composerDataHandle ?? data.getHandleIfLoaded(operation.id))?.data;
    if (!composer) return { reason: "composerMismatch", detail: "chat data unavailable" };
    if (composer.composerId !== undefined && composer.composerId !== operation.id) return { reason: "composerMismatch", detail: "composer id" };
    if (composer.isDraft === true) return { reason: "composerMismatch", detail: "chat is a draft" };
    if (composer.agentLocation && composer.agentLocation.type !== "local") return { reason: "locationMismatch", detail: String(composer.agentLocation.type) };
    const workspace = reference.workspaceIdentifier ?? composer.workspaceIdentifier;
    if (!workspace || workspace.id !== project.id) return { reason: "projectMismatch" };
    if (composer.unifiedMode !== "agent") return { reason: "modeMismatch", detail: String(composer.unifiedMode) };
    const actual = composer.modelConfig;
    if (!actual || typeof actual !== "object") return { reason: "modelMismatch", detail: "no model configuration" };
    if (operation.model) {
      if (actual.modelName !== config.modelName) return { reason: "modelMismatch", detail: String(actual.modelName) };
      const wanted = config.selectedModels?.[0], got = actual.selectedModels?.[0];
      if (wanted && got && (got.modelId !== wanted.modelId || !samePairs(got.parameters, wanted.parameters))) {
        return { reason: "modelMismatch", detail: "model parameters" };
      }
    }
    if (typeof operation.maxMode === "boolean" && actual.maxMode !== operation.maxMode) {
      return { reason: "maxModeMismatch", detail: String(actual.maxMode) };
    }
    if (operation.yolo ? composer.agentApprovalModeOverride !== "unrestricted" : composer.agentApprovalModeOverride === "unrestricted") {
      return { reason: "approvalMismatch", detail: String(composer.agentApprovalModeOverride) };
    }
    return { observed: { modelName: actual.modelName, maxMode: actual.maxMode, approval: composer.agentApprovalModeOverride ?? null } };
  }

  // The chat of a task that never reached a model is archived, never deleted.
  async function tidy(repo, id) {
    if (typeof repo.archiveAgent !== "function") return "unavailable";
    let timer;
    try {
      await Promise.race([repo.archiveAgent(id), new Promise((_, reject) => { timer = setTimeout(() => reject(new Error("timeout")), TIDY_MS); })]);
      return "archived";
    } catch { return "failed"; } finally { clearTimeout(timer); }
  }

  // The loaded chat stays referenced for a while so a task that is still starting is not evicted.
  function hold(reference) {
    if (typeof reference?.dispose !== "function") return;
    holds.add(reference);
    setTimeout(() => { holds.delete(reference); try { reference.dispose(); } catch {} }, HOLD_MS);
  }

  async function runCreate(operation) {
    const found = contract();
    if (found.reason) return failure(found.reason, found.detail);
    const project = findProject(found.metadata, [operation.cwd, ...(Array.isArray(operation.aliases) ? operation.aliases : [])]);
    if (project.error) return failure(project.error, project.detail);
    const built = buildModelConfig(found.models, operation);
    if (built.error) return failure(built.error, built.detail);
    const deadline = Number.isFinite(operation.deadline) && operation.deadline > 0 ? operation.deadline : Date.now() + DEFAULT_WINDOW_MS;
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), Math.max(1, deadline - Date.now()));
    let created = false, submitting = false, observed;
    const options = {
      composerId: operation.id,
      modelConfig: built.config,
      unifiedMode: "agent",
      destination: { type: "local" },
      trustPolicy: "require-trusted",
      skipFocusAfterSubmission: true,
      startupAbortSignal: controller.signal,
      onCreated: () => { created = true; },
      beforeInitialSubmit: async reference => {
        if (controller.signal.aborted || Date.now() > deadline) throw refusal("timeout");
        const verdict = verify(reference, operation, project, built.config);
        if (verdict.reason) throw refusal(verdict.reason, verdict.detail);
        observed = verdict.observed;
        if (operation.dryRun) throw dryRun;
        submitting = true;
      },
    };
    if (operation.yolo) options.agentApprovalModeOverride = "unrestricted";
    if (typeof operation.title === "string" && operation.title) options.promptPreview = operation.title.slice(0, 120);
    try {
      const reference = await found.repo.createAgent(operation.text, { type: "existing", environment: { id: project.id, uri: project.uri } }, options);
      hold(reference);
      return { status: "started", sessionId: operation.id, observed };
    } catch (error) {
      if (submitting) return { status: "unknown", reason: "controlUnknown", detail: brief(error) };
      const tidied = created ? await tidy(found.repo, operation.id) : "skipped";
      if (error === dryRun) return { status: "dryRun", observed, tidied };
      const info = error?.askhuman;
      if (info) return failure(info.reason, info.detail, { tidied });
      if (controller.signal.aborted || Date.now() > deadline) return failure("timeout", brief(error), { tidied });
      return failure("nativeUnavailable", brief(error), { tidied });
    } finally {
      clearTimeout(timer);
    }
  }

  const waitFor = entry => (entry.final ? Promise.resolve(entry.final) : new Promise(resolve => {
    const timer = setTimeout(() => resolve({ status: "pending" }), WAIT_MS);
    entry.promise.then(result => { clearTimeout(timer); resolve(result); });
  }));

  const state = {
    owner, revision, lease,
    status: () => ({ prepared: enabled, enabled, intact: enabled, verified: enabled, checks: 1, surface: "messages" }),
    activate() { return this.status(); },
    owns(id) { return enabled && !!data.getHandleIfLoaded(id); },
    isAgents,
    restore() {
      enabled = false;
      for (const reference of holds) { try { reference.dispose(); } catch {} }
      holds.clear();
      if (globalThis[key] === state) delete globalThis[key];
      return true;
    },
    // Answers within WAIT_MS: the final result when there is one, else `pending`. A caller that
    // asks again with the same operation joins the same creation; it never starts a second one.
    // Only a rejection forgets the operation, and a rejection is always "nothing was submitted".
    create(operation) {
      if (!enabled || !operation || typeof operation !== "object" || typeof operation.id !== "string" || !UUID.test(operation.id) ||
          typeof operation.cwd !== "string" || typeof operation.text !== "string" || !operation.text) {
        return Promise.resolve(failure("nativeUnavailable", "invalid operation"));
      }
      const body = JSON.stringify({ ...operation, deadline: 0 });
      let entry = creations.get(operation.id);
      if (entry && entry.body !== body) return Promise.resolve(failure("conflict"));
      if (!entry) {
        entry = { body, final: undefined };
        const created = entry;
        created.promise = runCreate(operation).catch(error => failure("nativeUnavailable", brief(error))).then(result => {
          created.final = result;
          if (result.status === "rejected" && creations.get(operation.id) === created) creations.delete(operation.id);
          return result;
        });
        creations.set(operation.id, created);
      }
      return waitFor(entry);
    },
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
