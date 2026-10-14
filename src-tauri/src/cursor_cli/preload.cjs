'use strict';
// Loaded only by the managed launcher; no Inspector or model-content access.
const fs = require('node:fs');
const path = require('node:path');
const Module = require('node:module');
const boot = process.env.ASKHUMAN_CURSOR_CLI_BOOT;
const originalOptions = process.env.ASKHUMAN_CURSOR_CLI_NODE_OPTIONS;
if (boot) {
  if (originalOptions === undefined) delete process.env.NODE_OPTIONS;
  else process.env.NODE_OPTIONS = originalOptions;
  delete process.env.ASKHUMAN_CURSOR_CLI_BOOT;
  delete process.env.ASKHUMAN_CURSOR_CLI_NODE_OPTIONS;
}

function attach(options, dependencies = {}) {
  const fileSystem = dependencies.fs || fs;
  const moduleApi = dependencies.module || Module;
  const state = { pid: process.pid, revision: options.revision, phase: 'starting', active: false,
    suppressed: 0, version: path.basename(path.dirname(options.index)), updatedAt: Date.now() };
  const registryId = './src/shared/cli-background-work-registry.ts';
  const guards = new Map();
  const factories = new Map();
  let enabled = false, closed = false;
  const startedAt = Date.now();
  const missingRegistry = 'Cursor CLI background registry was not found after startup';
  function save() {
    state.updatedAt = Date.now();
    try {
      const temp = options.status + '.tmp';
      fileSystem.writeFileSync(temp, JSON.stringify(state), {mode: 0o600});
      fileSystem.renameSync(temp, options.status);
    } catch { /* Diagnostics must never break the native CLI. */ }
  }
  function policy() {
    try {
      const value = JSON.parse(fileSystem.readFileSync(options.policy, 'utf8'));
      if (typeof value.enabled !== 'boolean') throw Error('Invalid policy');
      enabled = value.enabled;
      if (state.error === 'Unable to read Cursor CLI policy') delete state.error;
    } catch {
      // Retain the last explicit policy across a transient read failure.
      if (!state.error) state.error = 'Unable to read Cursor CLI policy';
      state.phase = 'error';
    }
  }
  function restore() {
    for (const [prototype, entry] of guards) {
      if (prototype.pull === entry.guard) Object.defineProperty(prototype, 'pull', entry.descriptor);
    }
    state.active = false;
  }
  function install(prototype) {
    if (!prototype || guards.has(prototype)) return;
    const descriptor = Object.getOwnPropertyDescriptor(prototype, 'pull');
    const ack = prototype.ack;
    if (!descriptor?.configurable || typeof descriptor.value !== 'function' ||
        !descriptor.value.toString().includes('super.pull') || typeof ack !== 'function' ||
        !ack.toString().includes('forgetInFlight') || typeof prototype.suppress !== 'function') {
      state.phase = 'incompatible'; state.error = 'Unsupported Cursor CLI background registry contract'; save(); return;
    }
    const guard = function (...args) {
      // Read at consumption time too: an explicit disable cannot consume one more batch.
      policy();
      const batch = descriptor.value.apply(this, args);
      if (!enabled || !batch.length) return batch;
      // Native ACK performs in-flight and durable completion bookkeeping. Never synthesize results.
      const completions = batch.filter(item => ['shell', 'subagent'].includes(item.payload?.kind));
      if (completions.length) {
        ack.call(this, completions.map(item => item.id));
        state.suppressed += completions.length;
        save();
      }
      return batch.filter(item => !completions.includes(item));
    };
    if (state.error === missingRegistry) delete state.error;
    guards.set(prototype, {descriptor, guard});
    if (enabled) Object.defineProperty(prototype, 'pull', {...descriptor, value: guard});
  }
  function wrapChunk(result) {
    const factory = result?.modules?.[registryId];
    if (typeof factory !== 'function' || factories.has(result.modules)) return;
    const wrapped = function (module, exports, webpack) {
      const result = factory.apply(this, arguments);
      install(exports.t?.prototype);
      refresh();
      return result;
    };
    factories.set(result.modules, {factory, wrapped});
    result.modules[registryId] = wrapped;
  }
  // Cursor bundles load lazy CJS chunks. Keep this internal Node hook narrowly scoped and
  // validate the vendor ABI before touching its prototype; incompatible versions stay usable.
  const originalLoad = moduleApi._load;
  const hook = function (request, parent, isMain) {
    const result = originalLoad.apply(this, arguments);
    try {
      const filename = moduleApi._resolveFilename(request, parent, isMain);
      if (typeof filename === 'string' && path.dirname(filename) === path.dirname(options.index) &&
          filename.endsWith('.index.js')) wrapChunk(result);
    } catch (error) { state.phase = 'error'; state.error = String(error.message).slice(0, 200); save(); }
    return result;
  };
  function refresh() {
    policy();
    if (!enabled) {
      restore();
      if (moduleApi._load === hook) moduleApi._load = originalLoad;
      for (const [modules, entry] of factories) if (modules[registryId] === entry.wrapped) modules[registryId] = entry.factory;
    } else {
      if (moduleApi._load === originalLoad) moduleApi._load = hook;
      for (const [modules, entry] of factories) if (modules[registryId] === entry.factory) modules[registryId] = entry.wrapped;
      for (const [prototype, entry] of guards) {
        if (prototype.pull === entry.descriptor.value) Object.defineProperty(prototype, 'pull', {...entry.descriptor, value: entry.guard});
        else if (prototype.pull !== entry.guard) {
          state.phase = 'conflict'; state.error = 'Cursor CLI registry changed after attachment';
        }
      }
    }
    if (enabled && !guards.size && Date.now() - startedAt > 30000 && !state.error) {
      state.phase = 'incompatible'; state.error = missingRegistry;
    }
    state.active = enabled && !state.error && guards.size > 0 && [...guards].every(([p, e]) => p.pull === e.guard);
    if (!state.error) state.phase = state.active ? 'active' : enabled ? 'starting' : 'disabled';
    save();
  }
  function close() {
    if (closed) return;
    closed = true;
    clearInterval(timer);
    restore();
    if (moduleApi._load === hook) moduleApi._load = originalLoad;
    for (const [modules, entry] of factories) if (modules[registryId] === entry.wrapped) modules[registryId] = entry.factory;
    try { fileSystem.unlinkSync(options.status); } catch { /* Already removed. */ }
  }
  policy();
  moduleApi._load = hook;
  const timer = setInterval(refresh, 1000);
  timer.unref();
  process.once('exit', close);
  save();
  return {state, refresh, close, wrapChunk};
}
module.exports = {attach};
if (boot) {
  try {
    const options = JSON.parse(boot);
    if (process.argv[1] && fs.realpathSync(process.argv[1]) === options.index &&
        fs.statSync(path.dirname(options.policy)).uid === process.getuid() &&
        (fs.statSync(path.dirname(options.policy)).mode & 0o077) === 0) attach(options);
  } catch { /* An invalid bootstrap leaves the original command usable. */ }
}
