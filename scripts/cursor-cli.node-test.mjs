import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, linkSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';
import vm from 'node:vm';
import preload from '../src-tauri/src/cli/cursor_preload.cjs';

const { patchStall, patchWake, stallPolicy, wrapStallClassifier } = preload;

test('the timeout policy preserves disabled detectors, warnings and longer thresholds', () => {
  const disabled = { stallDetectorConfig: { failTimeoutMs: 0 } };
  assert.deepEqual(stallPolicy(disabled), { options: disabled, windowMs: 0 });
  const settings = { stallDetectorConfig: { failTimeoutMs: 90000, advisoryTimeoutMs: 30000 } };
  assert.deepEqual(stallPolicy(settings), {
    options: { stallDetectorConfig: { failTimeoutMs: 300000, advisoryTimeoutMs: 30000 } },
    windowMs: 600000,
  });
  assert.equal(settings.stallDetectorConfig.failTimeoutMs, 90000);
  assert.equal(stallPolicy({ stallDetectorConfig: { failTimeoutMs: 720000 } })
    .options.stallDetectorConfig.failTimeoutMs, 720000);
});

test('the classifier wrapper retains native cancellation and retry accounting', () => {
  for (const originalWindow of [180000, 240000, 720000]) {
    const original = (error, state) => state.retries >= 10 || error === 'cancel'
      || (error === 'stall' && state.stallRetryElapsedMs >= originalWindow) ? 'stop' : 'retry';
    const guarded = wrapStallClassifier(original, 600000);
    const state = { stallRetryElapsedMs: 300000, retries: 0 };
    assert.equal(guarded('stall', state), 'retry');
    assert.equal(state.stallRetryElapsedMs, 300000);
    assert.equal(guarded('stall', { ...state, stallRetryElapsedMs: 599999 }), 'retry');
    assert.equal(guarded('stall', { ...state, stallRetryElapsedMs: 600000 }),
      originalWindow <= 600000 ? 'stop' : 'retry');
    assert.equal(guarded('cancel', state), 'stop');
    assert.equal(guarded('reset', { ...state, retries: 10 }), 'stop');
  }
});

test('the bundle transform sets both stall controls in the same turn', async () => {
  const source = `class Agent {
    async run(t,p){return retry({runOptions:p,requestId:t,notifyInitialResume:!0,logging:{},hooks:{
      beginAttempt:()=>{},classify:(e,s)=>s.stallRetryElapsedMs>=180000?'stop':'retry',buildNonRetriedErrorInfo:()=>null}})}
  }; Agent;`;
  const Agent = vm.runInNewContext(patchStall(source), {
    retry({ runOptions, hooks }) {
      return [runOptions.stallDetectorConfig.failTimeoutMs,
        hooks.classify('stall', { stallRetryElapsedMs: 300000 }),
        hooks.classify('stall', { stallRetryElapsedMs: 600000 })];
    },
  });
  assert.deepEqual(await new Agent().run('request', {}), [300000, 'retry', 'stop']);
  assert.throws(() => patchStall(source.replace('buildNonRetriedErrorInfo', 'otherHook')));
});

test('wake guard settles only eligible shell and subagent notifications', async () => {
  const source = `async function wake(t,held,ready){
    if(t.backgroundWorkRegistry.nack(held.map((x=>x.id)),{requeue:!0}),0===ready.length)return;try{const s={stack:[],error:void 0,hasError:!1};try{const p=ready.map((x=>x.payload));
      dispatch({case:"backgroundTaskCompletionAction",payload:p});
      t.backgroundWorkRegistry.ack(ready.map((x=>x.id)));
    }catch(e){s.error=e;s.hasError=!0}}catch(e){throw t.backgroundWorkRegistry.nack(ready.map((x=>x.id)),{requeue:!0}),e}
  }; wake;`;
  const observed = [];
  const wake = vm.runInNewContext(patchWake(source), { dispatch(action) { observed.push(action.payload); } });
  const registry = { nack(ids) { observed.push(['nack', ...ids]); }, ack(ids) { observed.push(['ack', ...ids]); } };
  const item = (id, kind) => ({ id, payload: { kind } });
  await wake({ backgroundWorkRegistry: registry }, [item('awaited', 'shell')],
    [item('shell', 'shell'), item('subagent', 'subagent')]);
  assert.deepEqual(observed, [['nack', 'awaited'], ['ack', 'shell', 'subagent']]);
  observed.length = 0;
  await wake({ backgroundWorkRegistry: registry }, [], [item('shell', 'shell'), item('other', 'other')]);
  assert.deepEqual(observed.map(x => Array.from(x)), [['nack'], ['ack', 'shell'], [{ kind: 'other' }], ['ack', 'other']]);
});

const candidate = process.env.ASKHUMAN_CURSOR_CLI_PACKAGE;
test('Max protection requires both resume routes and the model manager before startup', () => {
  const scratch = mkdtempSync(path.join(tmpdir(), 'askhuman-cursor-contract-'));
  try {
    writeFileSync(path.join(scratch, 'index.js'), 'throw new Error("must never run");');
    assert.throws(() => preload.findMaxResumePatch(scratch), /cannot identify the Cursor resume interfaces/);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test('the guarded vendor model manager uses a non-Max variant or preserves the current model', { skip: !candidate }, async () => {
  const directory = realpathSync(candidate);
  const { transformed } = preload.findMaxResumePatch(directory);
  const bundle = {};
  const warnings = [];
  vm.runInNewContext(transformed, {
    exports: bundle,
    process: { stderr: { write: warning => warnings.push(warning) }, exit: code => { throw new Error(`exit ${code}`); } },
  });
  class Proto {
    constructor(data) { Object.assign(this, data); }
  }
  const protocol = new Proxy({}, { get: () => Proto });
  const modules = new Map(Object.entries({
    '../proto/dist/generated/agent/v1/agent_pb.js': protocol,
    '../proto/dist/generated/agent/v1/requested_model_pb.js': protocol,
    './src/bedrock/model-utils.ts': { _n: () => false },
    './src/bedrock/profile.ts': { UN: () => ({ enabled: false }) },
    './src/console-io.ts': {},
    './src/debug.ts': { cY: () => {}, debugLog: () => {} },
    './src/hang-detection.ts': {},
    './src/models/model-service.ts': {},
  }));
  function load(id) {
    if (modules.has(id)) return modules.get(id);
    if (!bundle.modules[id]) {
      for (const file of readdirSync(directory).filter(name => /^\d+\.index\.js$/.test(name))) {
        const source = readFileSync(path.join(directory, file), 'utf8');
        if (!source.includes(`"${id}"(`)) continue;
        const exports = {};
        vm.runInNewContext(source, { exports });
        Object.assign(bundle.modules, exports.modules);
        break;
      }
    }
    assert.ok(bundle.modules[id], id);
    const exports = {};
    modules.set(id, exports);
    bundle.modules[id]({}, exports, load);
    return exports;
  }
  load.d = (exports, getters) => {
    for (const [key, get] of Object.entries(getters)) Object.defineProperty(exports, key, { get, enumerable: true });
  };
  const Manager = Object.values(load('./src/models/index.ts'))
    .find(value => value?.prototype?.setModelFromStoredId);
  assert.ok(Manager);

  const parameters = context => [{ id: 'context', value: context }];
  const small = { parameterValues: parameters('200k'), isDefaultNonMaxConfig: true, isMaxMode: false };
  const large = { parameterValues: parameters('1m'), isDefaultMaxConfig: true, isMaxMode: true };
  const catalog = [
    { name: 'safe', variants: [small], supportsNonMaxMode: true },
    { name: 'mixed', variants: [small, large], supportsNonMaxMode: true },
    { name: 'max-only', variants: [large], supportsNonMaxMode: false },
  ].map(model => ({
    ...model, legacySlugs: [], parameterDefinitions: [{
      id: 'context', parameterType: { enumParameter: { values: [{ value: '200k' }, { value: '1m' }] } },
    }],
  }));
  function modelState({ current = 'safe', activeContext = '200k', maxMode = false } = {}) {
    let settings = { maxMode, model: { modelId: current, maxMode },
      selectedModel: { modelId: current, parameters: parameters(activeContext) },
      modelParameters: { mixed: parameters('1m'), 'max-only': parameters('1m') } };
    const config = { get: () => settings, transform: async update => { settings = update(settings); } };
    const manager = new Manager(config);
    manager.parameterizedModels = catalog;
    manager.parameterizedModelMap = new Map(catalog.map(model => [model.name, model]));
    manager.hydrateCurrentModelFromConfig(config);
    return { manager, config };
  }

  let { manager, config } = modelState();
  assert.equal(await manager.setModelFromStoredId('mixed', config), true);
  assert.equal(config.get().maxMode, false);
  assert.equal(manager.buildRequestedModel().maxMode, false);
  assert.equal(manager.buildRequestedModel().parameters[0].value, '200k');
  assert.match(warnings.pop(), /non-Max variant of mixed/);

  ({ manager, config } = modelState());
  const before = config.get();
  assert.equal(await manager.setModelFromStoredId('max-only', config), false);
  assert.equal(config.get(), before);
  assert.equal(manager.buildRequestedModel().modelId, 'safe');
  assert.equal(manager.buildRequestedModel().maxMode, false);
  assert.match(warnings.pop(), /Keeping safe/);

  ({ manager, config } = modelState({ current: 'mixed', activeContext: '1m' }));
  const loadedModels = manager.parameterizedModelMap;
  manager.parameterizedModelMap = new Map();
  manager.models = Promise.resolve().then(() => { manager.parameterizedModelMap = loadedModels; });
  assert.equal(await manager.setModelFromStoredId('mixed', config), true);
  assert.equal(manager.buildRequestedModel().parameters[0].value, '200k');
  assert.equal(manager.buildRequestedModel().maxMode, false);
  warnings.length = 0;

  ({ manager, config } = modelState());
  assert.equal(await manager.setModelFromStoredId('safe', config), true);
  await manager.setCurrentModelWithParameters('max-only', parameters('1m'), config);
  assert.equal(config.get().maxMode, true);
  assert.equal(manager.buildRequestedModel().maxMode, true);
  assert.deepEqual(warnings, []);

  ({ manager, config } = modelState({ maxMode: true }));
  assert.equal(await manager.setModelFromStoredId('max-only', config), true);
  assert.equal(manager.buildRequestedModel().maxMode, true);
  assert.deepEqual(warnings, []);

  ({ manager, config } = modelState({ current: 'max-only', activeContext: '1m' }));
  await assert.rejects(manager.setModelFromStoredId('max-only', config), /exit 1/);
  assert.match(warnings.pop(), /no current non-Max model/);

  ({ manager, config } = modelState());
  manager.parameterizedModels = ['safe', 'fixed'].map(name => ({
    name, parameterDefinitions: [], variants: [{ parameterValues: [], isMaxMode: false }],
  }));
  manager.parameterizedModelMap = new Map(manager.parameterizedModels.map(model => [model.name, model]));
  assert.equal(await manager.setModelFromStoredId('fixed', config), true);
  assert.equal(manager.buildRequestedModel().maxMode, false);
  assert.deepEqual(warnings, []);
});

test('the vendor Node passes transformed sources to its loader without running Cursor', {
  skip: !candidate || !existsSync(path.join(candidate, 'node')),
}, t => {
  const directory = realpathSync(candidate);
  const runtime = path.join(directory, 'node');
  const version = spawnSync(runtime, ['--version'], { encoding: 'utf8', timeout: 10000 });
  if (version.status !== 0 && path.basename(directory).startsWith('.')) {
    t.skip('cached staging directory does not have a runnable vendor Node');
    return;
  }
  assert.equal(version.status, 0, version.stderr);
  const main = path.join(directory, 'index.js');
  const mainHash = createHash('sha256').update(readFileSync(main)).digest('hex');
  const build = preload.BUILDS[mainHash];
  const max = preload.findMaxResumePatch(directory);
  const model = max.asset[0];
  const interactive = max.routes[0][0];
  const scratch = mkdtempSync(path.join(tmpdir(), 'askhuman-cursor-loader-'));
  try {
    const observer = path.join(scratch, 'observer.cjs');
    const guard = fileURLToPath(new URL('../src-tauri/src/cli/cursor_preload.cjs', import.meta.url));
    writeFileSync(observer, `
      const assert = require('node:assert/strict');
      const { registerHooks } = require('node:module');
      assert.equal(process.env.ASKHUMAN_CURSOR_CLI_BOOT, undefined);
      assert.equal(process.env.NODE_OPTIONS, '--no-warnings');
      registerHooks({ load(url, context, nextLoad) {
        const result = nextLoad(url, context);
        if (url === ${JSON.stringify(pathToFileURL(main).href)}) {
          ${build ? 'assert.match(String(result.source), /ahStall\\.windowMs/);' : ''}
          return { ...result, source: ${JSON.stringify(`require('./${interactive}')`)} };
        }
        if (url === ${JSON.stringify(pathToFileURL(path.join(directory, interactive)).href)}) {
          ${build ? 'assert.match(String(result.source), /settleBackgroundCompletions/);' : ''}
          return { ...result, source: ${JSON.stringify(`require('./${model}')`)} };
        }
        if (url === ${JSON.stringify(pathToFileURL(path.join(directory, model)).href)}) {
          assert.match(String(result.source), /ahChooseNonMaxResume/);
          return { ...result, source: "console.log('loader check passed')" };
        }
        assert.ok(!url.startsWith(${JSON.stringify(pathToFileURL(directory + path.sep).href)}), url);
        return result;
      } });
    `);
    const result = spawnSync(runtime, ['--require', guard, '--require', observer, main], {
      env: { ...process.env, NODE_OPTIONS: '--no-warnings',
        ASKHUMAN_CURSOR_CLI_BOOT: JSON.stringify({ package: directory, originalOptions: '--no-warnings' }) },
      encoding: 'utf8', timeout: 10000,
    });
    assert.ifError(result.error);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, 'loader check passed\n');
    if (build) assert.equal(result.stderr, '');
    else assert.match(result.stderr, /stall timeout and background wake-up protection unavailable/);

    const fakeRuntime = path.join(scratch, 'node');
    const fakeMain = path.join(scratch, 'index.js');
    linkSync(runtime, fakeRuntime);
    writeFileSync(fakeMain, "console.log('unprotected Cursor ran')");
    const unsupported = spawnSync(fakeRuntime, ['--require', guard, fakeMain], {
      env: { ...process.env, ASKHUMAN_CURSOR_CLI_BOOT: JSON.stringify({ package: realpathSync(scratch), originalOptions: null }) },
      encoding: 'utf8', timeout: 10000,
    });
    assert.ifError(unsupported.error);
    assert.equal(unsupported.status, 1, unsupported.stderr);
    assert.equal(unsupported.stdout, '');
    assert.match(unsupported.stderr, /Max resume protection stopped Cursor.*cannot identify/);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});
