// Real-process isolation regression. No daemon, GUI, channel credentials or model calls.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createInterface } from 'node:readline';
import { randomUUID } from 'node:crypto';
import { sourceFingerprint, recordBuild } from './dev-build-record.mjs';

if (process.platform === 'win32') {
  console.log('SKIP: Windows uses Known Folder home; run native Rust tests without redirecting a real user profile.');
  process.exit(0);
}
const original = path.resolve(process.argv[2] ?? 'src-tauri/target/local-install/AskHuman');
assert.ok(fs.statSync(original).isFile(), 'pass a freshly built AskHuman executable');
const temp = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'askhuman-dev-routing-')));
const home = path.join(temp, 'user');
fs.mkdirSync(home);
const system = path.join(temp, 'system/AskHuman');
fs.mkdirSync(path.dirname(system));
fs.copyFileSync(original, system, fs.constants.COPYFILE_FICLONE);
fs.chmodSync(system, 0o700);
const env = { ...process.env, HOME: home, ASKHUMAN_NO_KEYCHAIN: '1' };
for (const name of Object.keys(env)) if (name.startsWith('ASKHUMAN_') && name !== 'ASKHUMAN_NO_KEYCHAIN') delete env[name];
let server;
const workspaces = [];
let donorProfile;
function run(args, cwd = temp, extra = {}, ok = true) {
  const output = spawnSync(system, args, { cwd, env: { ...env, ...extra }, encoding: 'utf8', timeout: 15000, maxBuffer: 4 * 1024 * 1024 });
  if (ok) assert.equal(output.status, 0, `${args.join(' ')}: ${output.stderr}`);
  else assert.notEqual(output.status, 0, 'expected an explicit failure');
  return output;
}
function json(args, cwd, extra) { return JSON.parse(run(args, cwd, extra).stdout); }
function worktree(name) {
  const root = path.join(temp, name);
  fs.mkdirSync(root);
  assert.equal(spawnSync('git', ['init', '--quiet', root]).status, 0);
  run(['dev', 'enable'], root);
  const binary = path.join(root, '.askhuman-dev/bin/AskHuman');
  fs.copyFileSync(original, binary, fs.constants.COPYFILE_FICLONE);
  fs.chmodSync(binary, 0o700);
  const build = recordBuild(root, binary, sourceFingerprint(root));
  return { root, binary, build };
}

try {
  run(['dev', 'register-system', system]);
  const a = worktree('tree-a'), b = worktree('tree-b');
  fs.renameSync(system, `${system}.image`);
  fs.symlinkSync(`${system}.image`, system);
  run(['dev', 'register-system', system]);
  assert.equal(JSON.parse(fs.readFileSync(path.join(home, '.askhuman/development/routing.json'))).systemBinary, system, 'registration must retain the stable entry');
  const routingFile = path.join(home, '.askhuman/development/routing.json');
  const routing = JSON.parse(fs.readFileSync(routingFile));
  routing.mode = 'explicit';
  fs.writeFileSync(routingFile, JSON.stringify(routing));
  assert.equal(json(['dev', 'target'], a.root).kind, 'system');
  for (const tree of [a, b]) {
    const target = json(['--instance', 'dev', '--worktree', tree.root, 'dev', 'target'], tree.root);
    assert.equal(target.kind, 'dev');
    assert.equal(target.home, path.join(tree.root, '.askhuman-dev/home'));
  }
  assert.equal(json(['dev', 'target'], b.root).kind, 'system');
  run(['--instance', 'system', 'dev', 'target'], a.root, { ASKHUMAN_HOME: path.join(a.root, '.askhuman-dev/home') }, false);
  const pinned = json(['--instance', 'system', 'dev', 'target'], a.root);
  const inherited = { ASKHUMAN_RESOLVED_TARGET_V1: JSON.stringify(pinned) };
  assert.equal(json(['dev', 'target'], b.root, inherited).kind, 'system');
  run(['--instance', 'dev', '--worktree', a.root, 'dev', 'target'], a.root, inherited, false);
  run(['--instance', 'system', 'todo', 'add', 'formal-sentinel'], a.root);
  run(['--instance', 'dev', '--worktree', a.root, 'todo', 'add', 'dev-sentinel'], a.root);

  server = spawn(system, ['--instance', 'system', 'mcp'], { cwd: a.root, env, stdio: ['pipe', 'pipe', 'pipe'] });
  server.stderr.resume();
  const awaiting = new Map();
  const lines = createInterface({ input: server.stdout });
  lines.on('line', line => { const response = JSON.parse(line); if (response.id != null) { awaiting.get(response.id)?.(response); awaiting.delete(response.id); } });
  let id = 0;
  const call = (method, params) => new Promise((resolve, reject) => {
    const requestId = ++id;
    const timer = setTimeout(() => { awaiting.delete(requestId); reject(new Error(`MCP ${method} timed out`)); }, 15000);
    awaiting.set(requestId, response => { clearTimeout(timer); if (response.error) reject(new Error(JSON.stringify(response.error))); else resolve(response.result); });
    server.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: requestId, method, params }) + '\n');
  });
  await call('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'isolation-smoke', version: '1' } });
  server.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
  const catalog = await call('tools/list', {});
  assert.ok(catalog.tools.some(tool => tool.name === 'dev_ask'));
  const todos = () => call('tools/call', { name: 'todo_list', arguments: {} });
  assert.match(JSON.stringify(await todos()), /formal-sentinel/);
  fs.unlinkSync(path.join(a.root, '.askhuman-dev/enabled'));
  assert.match(JSON.stringify(await todos()), /formal-sentinel/);
  assert.doesNotMatch(JSON.stringify(await todos()), /dev-sentinel/);
  fs.writeFileSync(path.join(a.root, '.askhuman-dev/enabled'), '');

  const verificationId = randomUUID();
  const verificationDirectory = path.join(a.root, '.askhuman-dev/home/state/verifications');
  fs.mkdirSync(verificationDirectory, { recursive: true });
  fs.writeFileSync(path.join(verificationDirectory, `${verificationId}.json`), JSON.stringify({ version: 1, info: { id: verificationId, worktree: a.root, build: a.build, delivery: 'local', relay: false }, requestFingerprint: 'fixture', status: 'completed', executorPid: process.pid, stdout: '[user_input]\nverified-fixture-answer', exitCode: 0, createdAtMs: 0, updatedAtMs: 0 }));
  const answer = await call('tools/call', { name: 'dev_ask', arguments: { worktree: a.root, resume: verificationId } });
  assert.match(JSON.stringify(answer), /verified-fixture-answer/);
  assert.match(JSON.stringify(answer), /Dev verification metadata/);
  assert.match(JSON.stringify(await todos()), /formal-sentinel/);
  assert.doesNotMatch(JSON.stringify(await todos()), /dev-sentinel/);
  const changed = { ...a.build, binaryFingerprint: 'another-build' };
  fs.writeFileSync(path.join(verificationDirectory, `${verificationId}.json`), JSON.stringify({ version: 1, info: { id: verificationId, worktree: a.root, build: changed, delivery: 'local', relay: false }, requestFingerprint: 'fixture', status: 'completed', executorPid: process.pid, stdout: 'stale-answer', exitCode: 0, createdAtMs: 0, updatedAtMs: 0 }));
  assert.ok((await call('tools/call', { name: 'dev_ask', arguments: { worktree: a.root, resume: verificationId } })).isError);
  server.stdin.end(); server.kill(); server = null;

  fs.unlinkSync(path.join(b.root, '.askhuman-dev/enabled'));
  run(['--instance', 'dev', '--worktree', b.root, 'dev', 'target'], temp, {}, false);
  fs.writeFileSync(path.join(b.root, '.askhuman-dev/enabled'), '');
  fs.chmodSync(b.binary, 0o600);
  run(['--instance', 'dev', '--worktree', b.root, 'dev', 'target'], temp, {}, false);
  fs.chmodSync(b.binary, 0o700);
  fs.mkdirSync(path.join(b.root, 'src'));
  fs.writeFileSync(path.join(b.root, 'src/changed.rs'), '// unbuilt source');
  assert.match(run(['dev', 'ask', '--worktree', b.root, '--', 'must not show a question'], temp, {}, false).stderr, /source|rebuild/);
  fs.rmSync(path.join(b.root, 'src'), {recursive:true});
  fs.writeFileSync(b.binary, '#!/bin/sh\nprintf old-client\n');
  recordBuild(b.root, b.binary, sourceFingerprint(b.root));
  assert.match(run(['dev', 'ask', '--worktree', b.root, '--', 'must not show a question'], temp, {}, false).stderr, /does not support/);
  fs.copyFileSync(original, b.binary);fs.chmodSync(b.binary,0o700);
  recordBuild(b.root,b.binary,sourceFingerprint(b.root));

  // Preparing native profiles must only write private files; no Agent/model session is started.
  const globalBefore = new Map(['.claude.json', '.claude/settings.json', '.codex/config.toml', '.codex/hooks.json', '.grok/config.toml', '.pi/agent/extensions/askhuman/index.ts'].map(file => [file, fs.existsSync(path.join(home, file)) ? fs.readFileSync(path.join(home, file)) : null]));
  for (const kind of ['claude', 'codex', 'grok', 'pi']) {
    const prepared = spawnSync(system, ['dev', 'session', kind, '--worktree', a.root, '--prepare-only'], {cwd:temp,env,encoding:'utf8',timeout:30000});
    if (prepared.status !== 0 && /not installed/.test(prepared.stderr)) continue;
    assert.equal(prepared.status, 0, prepared.stderr);
    const profile = JSON.parse(prepared.stdout);
    donorProfile ??= profile;
    workspaces.push(profile.neutralWorkspace);
    assert.ok(profile.directory.startsWith(path.join(a.root, '.askhuman-dev/clients/')));
    assert.equal(profile.prepared, true);
    assert.ok(fs.existsSync(path.join(profile.neutralWorkspace, 'askhuman-owner.json')));
    assert.equal(fs.existsSync(path.join(profile.directory, 'auth.json')), false);
    if (kind === 'codex') assert.match(fs.readFileSync(path.join(profile.directory, 'config.toml'), 'utf8'), /ASKHUMAN_DEV_TEST_PROFILE/);
  }
  if (donorProfile) {
    const target = json(['--instance', 'dev', '--worktree', a.root, 'dev', 'target'], a.root);
    const rejected = run(['dev', 'ask', '--worktree', b.root, '--', '--invalid-routing-smoke-option'], a.root, {
      ASKHUMAN_RESOLVED_TARGET_V1: JSON.stringify(target), ASKHUMAN_HOME: target.home,
      ASKHUMAN_DEV_TEST_PROFILE: donorProfile.id,
    }, false);
    assert.doesNotMatch(rejected.stderr, /profile|test-profiles/);
    assert.match(rejected.stderr, /invalid-routing-smoke-option/);
  }
  for (const [file, bytes] of globalBefore) assert.deepEqual(fs.existsSync(path.join(home, file)) ? fs.readFileSync(path.join(home, file)) : null, bytes);

  // Seed genuine legacy bundles, then migrate every adapter without touching a real home.
  routing.mode = 'legacy'; fs.writeFileSync(routingFile, JSON.stringify(routing));
  for (const [agent, mode] of [['claude', 'mcp'], ['codex', 'mcp'], ['cursor', 'mcp'], ['grok', 'mcp'], ['pi', 'cli'], ['antigravity', 'mcp']]) {
    run(['--instance', 'system', 'agents', 'mode', agent, mode]);
  }
  const plan = json(['--instance', 'system', 'dev', 'migrate', 'plan']);
  assert.deepEqual(plan.conflicts, []);
  const before = new Map(plan.artifacts.filter(item => fs.existsSync(item.path)).map(item => [item.path, fs.readFileSync(item.path)]));
  const applied = json(['--instance', 'system', 'dev', 'migrate', 'apply', plan.id]);
  assert.equal(applied.phase, 'complete', applied.error ?? JSON.stringify(applied.actions));
  assert.equal(JSON.parse(fs.readFileSync(routingFile)).mode, 'explicit');
  assert.match(fs.readFileSync(path.join(home, '.codex/config.toml'), 'utf8'), /"--instance", "system", "mcp"/);
  assert.match(fs.readFileSync(path.join(home, '.claude/settings.json'), 'utf8'), /--instance system/);
  const codex = path.join(home, '.codex/config.toml');
  const migrated = fs.readFileSync(codex);
  fs.appendFileSync(codex, '\n# concurrent custom edit\n');
  run(['--instance', 'system', 'dev', 'migrate', 'rollback', plan.id], temp, {}, false);
  assert.match(fs.readFileSync(codex, 'utf8'), /concurrent custom edit/);
  fs.writeFileSync(codex, migrated);
  assert.equal(json(['--instance', 'system', 'dev', 'migrate', 'rollback', plan.id]).phase, 'rolledBack');
  assert.equal(JSON.parse(fs.readFileSync(routingFile)).mode, 'legacy');
  for (const [file, bytes] of before) assert.deepEqual(fs.readFileSync(file), bytes, `rollback must restore ${file}`);
  for (const agent of ['claude', 'codex', 'cursor', 'antigravity']) run(['--instance', 'system', 'agents', 'mode', agent, 'cli']);
  const cliPlan = json(['--instance', 'system', 'dev', 'migrate', 'plan']);
  assert.deepEqual(cliPlan.conflicts, []);
  const cliBefore = new Map(cliPlan.artifacts.filter(item => fs.existsSync(item.path)).map(item => [item.path, fs.readFileSync(item.path)]));
  const codexDirectory = path.join(home, '.codex');
  fs.chmodSync(codexDirectory, 0o500);
  const partial = json(['--instance', 'system', 'dev', 'migrate', 'apply', cliPlan.id]);
  assert.equal(partial.phase, 'partial');
  assert.ok(partial.actions.some(action => action.status === 'complete'));
  assert.ok(partial.actions.some(action => action.status === 'failed'));
  run(['--instance', 'system', 'dev', 'migrate', 'plan'], temp, {}, false);
  fs.chmodSync(codexDirectory, 0o700);
  assert.equal(json(['--instance', 'system', 'dev', 'migrate', 'apply', cliPlan.id]).phase, 'complete');
  assert.match(fs.readFileSync(path.join(home, '.claude/CLAUDE.md'), 'utf8'), /AskHuman --instance system/);
  assert.equal(json(['--instance', 'system', 'dev', 'migrate', 'rollback', cliPlan.id]).phase, 'rolledBack');
  for (const [file, bytes] of cliBefore) assert.deepEqual(fs.readFileSync(file), bytes);
  console.log('PASS: two worktrees; system/Dev/system; fixed MCP across marker changes; Dev resume and stale build rejection; six legacy bundles and CLI variants migrated; partial failure retried; external edit preserved; exact rollback');
} finally {
  server?.kill();
  for (const workspace of workspaces) {
    const owner = JSON.parse(fs.readFileSync(path.join(workspace, "askhuman-owner.json")));
    if (owner.worktree.startsWith(temp + path.sep)) fs.rmSync(workspace, { recursive: true, force: true });
  }
  fs.rmSync(temp, { recursive: true, force: true });
}
