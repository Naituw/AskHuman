// The Agents-window task creation of the Cursor message adapter, against synthetic object graphs.
// Nothing here starts Cursor, an Agent, or a model: the "repository" below only records what it was
// asked to do and mimics the order of Cursor's own `createAgent` (create, `onCreated`,
// `beforeInitialSubmit`, submit).
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import { test } from 'node:test';

const dir = new URL('../src-tauri/src/cursor_desktop/', import.meta.url);
const template = fs.readFileSync(new URL('message_renderer.js', dir), 'utf8');
const dispatch = fs.readFileSync(new URL('message_dispatch.js', dir), 'utf8');

const ID = '3f2b1c0e-5a4d-4e6f-8a7b-9c0d1e2f3a4b';
const PROJECT = '/work/demo';
const plain = value => JSON.parse(JSON.stringify(value));

function fixture(options = {}) {
  const log = { created: [], submitted: [], cleaned: 0, archived: [], disposed: 0 };
  const uri = { scheme: 'file', fsPath: options.projectPath ?? PROJECT, path: options.projectPath ?? PROJECT };
  const records = options.records ?? [{ workspaceId: 'ws-demo', paths: [{ uri }] }];
  const metadata = {
    getAllMetadata: () => records,
    findCanonicalIdByFolderUri: folder => (options.canonicalOf ? options.canonicalOf(folder) : (options.canonical ?? 'ws-canonical')),
  };
  const models = {
    getModelConfig: () => ({ modelName: 'default', maxMode: options.baseMax ?? false, selectedModels: [{ modelId: 'default', parameters: [] }] }),
    fixupModelConfigForCurrentFlag: config => config,
  };
  const repo = {
    async createAgent(text, environment, opts) {
      log.created.push({ text, environment, opts });
      if (options.createDelay) await new Promise(resolve => setTimeout(resolve, options.createDelay));
      if (options.createThrows) throw new Error(options.createThrows);
      const workspace = options.workspace ?? environment.environment;
      const composer = {
        composerId: options.composerId ?? opts.composerId,
        isDraft: options.isDraft,
        unifiedMode: options.unifiedMode ?? opts.unifiedMode,
        agentLocation: options.location ?? { type: 'local' },
        workspaceIdentifier: workspace,
        agentApprovalModeOverride: options.approval ?? opts.agentApprovalModeOverride,
        modelConfig: options.mutate ? options.mutate(plain(opts.modelConfig)) : opts.modelConfig,
      };
      const reference = {
        header: { id: options.headerId ?? opts.composerId },
        composerDataHandle: { data: composer },
        workspaceIdentifier: workspace,
        dispose() { log.disposed++; },
      };
      opts.onCreated?.(reference);
      try {
        await opts.beforeInitialSubmit?.(reference);
      } catch (error) {
        log.cleaned++; // Cursor discards the chat when this hook throws.
        throw error;
      }
      log.submitted.push(text);
      if (options.submitThrows) throw new Error('submit failed');
      return reference;
    },
    async archiveAgent(id) {
      if (options.archiveThrows) throw new Error('cannot archive');
      log.archived.push(id);
    },
  };
  const chat = {
    acquirePreGenerationSubmission() {}, isQuietForBackgroundSubmit() { return true; }, _submitChatMaybeAbortCurrent() {},
  };
  const entries = [
    ['composerChatService', chat],
    ...(options.noRepo ? [] : [['agentRepositoryService', repo]]),
    ['workspaceMetadataService', metadata],
    ['modelConfigService', models],
  ];
  const data = {
    _instantiationService: { _services: { _entries: entries } },
    _environmentService: { isGlass: options.glass !== false },
    getHandleIfLoaded: () => undefined,
  };
  // The adapter's long hold on a started chat must not keep the test process alive.
  const timers = (callback, ms) => {
    const timer = setTimeout(callback, ms);
    if (ms > 60000) timer.unref();
    return timer;
  };
  const context = vm.createContext({ setTimeout: timers, clearTimeout, AbortController, data });
  const source = template.replace('__ASKHUMAN_OWNER__', '"home"').replace('__ASKHUMAN_REVISION__', '"revision"').replace('__ASKHUMAN_LEASE__', '"lease"');
  vm.runInContext(`(function(){ return ${source}; }).call(data)`, context);
  return { state: context.__ASKHUMAN_CURSOR_MESSAGES_V1__, log };
}

const op = (extra = {}) => ({ id: ID, cwd: PROJECT, text: 'Build it', ...extra });
const create = async (f, extra) => plain(await f.state.create(op(extra)));

test('a task is created in the Agents window with the options the spec lists, and submitted once', async () => {
  const f = fixture();
  const result = await create(f, { title: 'Demo task' });
  assert.equal(result.status, 'started');
  assert.equal(result.sessionId, ID);
  assert.equal(f.log.created.length, 1);
  assert.equal(f.log.submitted.length, 1);
  const { text, environment, opts } = f.log.created[0];
  assert.equal(text, 'Build it');
  assert.equal(environment.type, 'existing');
  assert.equal(environment.environment.id, 'ws-canonical');
  assert.equal(environment.environment.uri.fsPath, PROJECT);
  assert.equal(opts.composerId, ID);
  assert.equal(opts.unifiedMode, 'agent');
  assert.equal(opts.destination.type, 'local');
  assert.equal(opts.trustPolicy, 'require-trusted');
  assert.equal(opts.skipFocusAfterSubmission, true);
  assert.equal(opts.promptPreview, 'Demo task');
  assert.equal('agentApprovalModeOverride' in opts, false);
  assert.equal(opts.modelConfig.modelName, 'default');
  assert.equal(opts.modelConfig.maxMode, false);
});

test('the live default configuration is the base and only the asked-for fields change', async () => {
  const f = fixture({ baseMax: true });
  assert.equal((await create(f)).status, 'started');
  assert.equal(f.log.created[0].opts.modelConfig.maxMode, true, 'Max untouched when the task says nothing');
  const g = fixture({ baseMax: true });
  assert.equal((await create(g, { maxMode: false })).status, 'started');
  assert.equal(g.log.created[0].opts.modelConfig.maxMode, false);
  const h = fixture();
  const model = { modelName: 'claude-4.5-sonnet', parameters: [{ id: 'thinking', value: 'true' }] };
  const result = await create(h, { model, maxMode: false });
  assert.equal(result.status, 'started');
  assert.equal(result.observed.modelName, 'claude-4.5-sonnet');
  const config = h.log.created[0].opts.modelConfig;
  assert.equal(config.modelName, 'claude-4.5-sonnet');
  assert.equal(config.selectedModels.length, 1);
  assert.equal(config.selectedModels[0].modelId, 'claude-4.5-sonnet');
  assert.equal(config.selectedModels[0].parameters[0].id, 'thinking');
  assert.equal(config.maxMode, false);
});

test('yolo sets the per-chat override and the readback insists on it', async () => {
  const f = fixture();
  assert.equal((await create(f, { yolo: true })).status, 'started');
  assert.equal(f.log.created[0].opts.agentApprovalModeOverride, 'unrestricted');
  const dropped = fixture({ approval: 'allowlist' });
  const result = await create(dropped, { yolo: true });
  assert.equal(result.status, 'rejected');
  assert.equal(result.reason, 'approvalMismatch');
  assert.equal(dropped.log.submitted.length, 0);
  // A chat that came out unrestricted when it was not asked to is just as wrong.
  const stray = fixture({ approval: 'unrestricted' });
  const strayResult = await create(stray);
  assert.equal(strayResult.reason, 'approvalMismatch');
  assert.equal(stray.log.submitted.length, 0);
});

test('every readback difference stops the task before anything is submitted and archives the chat', async () => {
  const cases = [
    ['composerMismatch', { headerId: 'someone-else' }],
    ['composerMismatch', { composerId: 'someone-else' }],
    ['composerMismatch', { isDraft: true }],
    ['locationMismatch', { location: { type: 'worktree' } }],
    ['projectMismatch', { workspace: { id: 'another-project' } }],
    ['modeMismatch', { unifiedMode: 'ask' }],
    ['maxModeMismatch', { mutate: config => ({ ...config, maxMode: true }), extra: { maxMode: false } }],
    ['modelMismatch', { mutate: config => ({ ...config, modelName: 'other' }), extra: { model: { modelName: 'gpt-5', parameters: [] } } }],
    ['modelMismatch', {
      mutate: config => ({ ...config, selectedModels: [{ modelId: 'gpt-5', parameters: [{ id: 'effort', value: 'low' }] }] }),
      extra: { model: { modelName: 'gpt-5', parameters: [{ id: 'effort', value: 'high' }] } },
    }],
  ];
  for (const [reason, { extra, ...options }] of cases) {
    const f = fixture(options);
    const result = await create(f, extra);
    assert.equal(result.status, 'rejected', reason);
    assert.equal(result.reason, reason);
    assert.equal(f.log.submitted.length, 0, `${reason} must not submit`);
    assert.equal(f.log.cleaned, 1, `${reason}: Cursor discards the chat`);
    assert.deepEqual(f.log.archived, [ID], `${reason}: the leftover chat is archived`);
    assert.equal(result.tidied, 'archived');
  }
});

test('a dry run verifies everything, submits nothing and leaves nothing behind', async () => {
  const f = fixture();
  const result = await create(f, { dryRun: true, maxMode: false });
  assert.equal(result.status, 'dryRun');
  assert.equal(result.observed.maxMode, false);
  assert.equal(f.log.submitted.length, 0);
  assert.equal(f.log.cleaned, 1);
  assert.deepEqual(f.log.archived, [ID]);
  // The same dry run answers again without creating a second chat.
  assert.equal((await create(f, { dryRun: true, maxMode: false })).status, 'dryRun');
  assert.equal(f.log.created.length, 1);
});

test('a dry run that would have been refused is refused, not reported as fine', async () => {
  const f = fixture({ mutate: config => ({ ...config, maxMode: true }) });
  const result = await create(f, { dryRun: true, maxMode: false });
  assert.equal(result.status, 'rejected');
  assert.equal(result.reason, 'maxModeMismatch');
});

test('the structural preflight refuses before anything is created', async () => {
  const reasons = [
    ['notAgentsWindow', { glass: false }],
    ['nativeUnavailable', { noRepo: true }],
    ['projectNotFound', { projectPath: '/elsewhere' }],
    ['projectNotFound', { records: [] }],
    ['projectNotFound', { records: [{ workspaceId: 'x', configPath: { path: '/w.code-workspace' }, paths: [{ uri: { scheme: 'file', fsPath: PROJECT } }] }] }],
    ['projectNotFound', { records: [{ workspaceId: 'x', paths: [{ uri: { scheme: 'vscode-remote', fsPath: PROJECT } }] }] }],
  ];
  for (const [reason, options] of reasons) {
    const f = fixture(options);
    const result = await create(f);
    assert.equal(result.status, 'rejected', JSON.stringify(options));
    assert.equal(result.reason, reason, JSON.stringify(options));
    assert.equal(f.log.created.length, 0, `${reason} must not create anything`);
  }
});

test('two different projects behind one path are ambiguous, one project listed twice is not', async () => {
  const one = { scheme: 'file', fsPath: PROJECT }, two = { scheme: 'file', fsPath: PROJECT };
  const records = [{ workspaceId: 'a', paths: [{ uri: one }] }, { workspaceId: 'b', paths: [{ uri: two }] }];
  const twice = fixture({ records, canonicalOf: () => 'same' });
  assert.equal((await create(twice)).status, 'started');
  const split = fixture({ records, canonicalOf: folder => (folder === one ? 'a' : 'b') });
  const result = await create(split);
  assert.equal(result.status, 'rejected');
  assert.equal(result.reason, 'projectAmbiguous');
  assert.equal(split.log.created.length, 0);
});

test('the project is found by its real path or by an alias, with or without a trailing slash', async () => {
  const f = fixture({ projectPath: '/real/demo' });
  assert.equal((await create(f, { cwd: '/link/demo', aliases: ['/real/demo/'] })).status, 'started');
  const g = fixture({ projectPath: '/real/demo' });
  assert.equal((await create(g, { cwd: '/link/demo' })).reason, 'projectNotFound');
  const h = fixture({ projectPath: '/real/demo' });
  assert.equal((await create(h, { cwd: 'relative/path' })).reason, 'projectNotFound');
});

test('invalid operations are refused up front', async () => {
  const f = fixture();
  for (const bad of [{ id: 'not-a-uuid' }, { text: '' }, { cwd: 5 }, null]) {
    const result = plain(await f.state.create(bad === null ? null : op(bad)));
    assert.equal(result.status, 'rejected');
  }
  assert.equal(f.log.created.length, 0);
});

test('an error before the submission is a refusal; after it the outcome is unknown and never retried', async () => {
  const early = fixture({ createThrows: 'service not ready' });
  const refused = await create(early);
  assert.equal(refused.status, 'rejected');
  assert.equal(refused.reason, 'nativeUnavailable');
  assert.match(refused.detail, /service not ready/);
  assert.equal(early.log.submitted.length, 0);

  const late = fixture({ submitThrows: true });
  const unknown = await create(late);
  assert.equal(unknown.status, 'unknown');
  assert.equal(late.log.archived.length, 0, 'a chat that may have reached a model is never touched');
  assert.equal((await create(late)).status, 'unknown');
  assert.equal(late.log.created.length, 1, 'the unknown outcome is remembered, not retried');
});

test('a deadline that passed before the check never submits', async () => {
  const f = fixture();
  const result = await create(f, { deadline: Date.now() - 1000 });
  assert.equal(result.status, 'rejected');
  assert.equal(result.reason, 'timeout');
  assert.equal(f.log.submitted.length, 0);
});

test('asking again with the same operation joins it; a changed one conflicts; a refusal can be retried', async () => {
  const f = fixture({ createDelay: 30 });
  const [a, b] = await Promise.all([f.state.create(op()), f.state.create(op({ deadline: Date.now() + 99999 }))]);
  assert.equal(plain(a).status, 'started');
  assert.equal(plain(b).status, 'started');
  assert.equal(f.log.created.length, 1);
  assert.equal(plain(await f.state.create(op())).status, 'started');
  assert.equal(plain(await f.state.create(op({ text: 'changed' }))).reason, 'conflict');

  const refused = fixture({ glass: false });
  assert.equal((await create(refused)).reason, 'notAgentsWindow');
  assert.equal((await create(refused)).reason, 'notAgentsWindow');
});

test('a slow creation answers pending and the next ask gets the result', async () => {
  const f = fixture({ createDelay: 2300 });
  const first = plain(await f.state.create(op()));
  assert.equal(first.status, 'pending');
  const second = plain(await f.state.create(op()));
  assert.equal(second.status, 'started');
  assert.equal(f.log.created.length, 1);
  assert.equal(f.log.submitted.length, 1);
});

test('the started chat stays referenced until the adapter is restored', async () => {
  const f = fixture();
  assert.equal((await create(f)).status, 'started');
  assert.equal(f.log.disposed, 0);
  f.state.restore();
  assert.equal(f.log.disposed, 1);
  assert.equal(plain(await f.state.create(op({ id: '4f2b1c0e-5a4d-4e6f-8a7b-9c0d1e2f3a4b' }))).status, 'rejected');
});

test('only a window that is the Agents window says so', () => {
  assert.equal(fixture().state.isAgents(), true);
  assert.equal(fixture({ glass: false }).state.isAgents(), false);
});

// ---- the bridge side: which window gets the creation ----

const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const run = new AsyncFunction('request', 'windows', 'refresh', 'evaluate', 'rendererKey', `let result; if (false) {} ${dispatch}\nreturn result;`);

function windowsOf(...agents) {
  const windows = new Map();
  agents.forEach((isAgents, index) => {
    windows.set(index, { phase: 'active', wc: { id: index, isDestroyed: () => false, isAgents } });
  });
  return windows;
}

async function dispatchCreate(windows, forwarded = []) {
  const evaluate = async (wc, expression) => {
    if (expression.includes('isAgents')) return wc.isAgents;
    forwarded.push([wc.id, expression]);
    return { status: 'started' };
  };
  return run({ op: 'create', operation: { id: ID, text: 'x' } }, windows, async () => {}, evaluate, '__KEY__');
}

test('the creation goes to exactly one Agents window', async () => {
  const forwarded = [];
  assert.equal((await dispatchCreate(windowsOf(false, true, false), forwarded)).status, 'started');
  assert.equal(forwarded.length, 1);
  assert.equal(forwarded[0][0], 1);
  assert.match(forwarded[0][1], /\.create\(/);
  assert.match(forwarded[0][1], new RegExp(ID));
});

test('with no Agents window, or several, nothing is created', async () => {
  const forwarded = [];
  assert.equal((await dispatchCreate(windowsOf(false, false), forwarded)).reason, 'noAgentsWindow');
  assert.equal((await dispatchCreate(windowsOf(true, true), forwarded)).reason, 'multipleAgentsWindows');
  assert.equal((await dispatchCreate(windowsOf(), forwarded)).reason, 'noAgentsWindow');
  assert.equal(forwarded.length, 0);
});

test('windows that are not active are never asked', async () => {
  const windows = windowsOf(true);
  windows.get(0).phase = 'error';
  assert.equal((await dispatchCreate(windows)).reason, 'noAgentsWindow');
});
