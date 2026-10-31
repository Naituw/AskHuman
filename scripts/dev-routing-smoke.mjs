// Real-process isolation regression with private daemons. No GUI, channel credentials or model calls.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createInterface } from 'node:readline';
import { createConnection } from 'node:net';
import { createHash, randomUUID } from 'node:crypto';
import { sourceFingerprint, recordBuild } from './dev-build-record.mjs';

if (process.platform === 'win32') {
  console.log('SKIP: Windows uses Known Folder home; run native Rust tests without redirecting a real user profile.');
  process.exit(0);
}
const original = path.resolve(process.argv[2] ?? 'src-tauri/target/local-install/AskHuman');
assert.ok(fs.statSync(original).isFile(), 'pass a freshly built AskHuman executable');
const temp = fs.realpathSync(fs.mkdtempSync(path.join(process.platform === 'darwin' ? '/tmp' : os.tmpdir(), 'ahdr-')));
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
const homes = [path.join(home, ".askhuman")];
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
  fs.mkdirSync(path.join(root, 'scripts/development'), { recursive: true });
  for (const name of ['dev.mjs', 'development/runtime.mjs', 'development/mcp.mjs', 'development/caller.mjs']) fs.copyFileSync(path.resolve('scripts', name), path.join(root, 'scripts', name));
  run(['dev', 'enable', '--agent', 'codex'], root);
  const before = fs.readFileSync(path.join(root, '.codex/config.toml'));
  run(['dev', 'enable', '--agent', 'codex'], root);
  assert.deepEqual(fs.readFileSync(path.join(root, '.codex/config.toml')), before);
  const instanceHome = path.join(root, '.askhuman-dev/home');
  homes.push(instanceHome);
  const config = JSON.parse(fs.readFileSync(path.join(instanceHome, 'config.json')));
  config.general.popupPrewarm = false; config.general.menuBarIcon = 'off';
  config.channels.popup.enabled = false;
  fs.writeFileSync(path.join(instanceHome, 'config.json'), JSON.stringify(config));
  const binary = path.join(root, '.askhuman-dev/bin/AskHuman');
  fs.copyFileSync(original, binary, fs.constants.COPYFILE_FICLONE);
  fs.chmodSync(binary, 0o700);
  const build = recordBuild(root, binary, sourceFingerprint(root));
  return { root, binary, build };
}

try {
  fs.mkdirSync(path.join(home, '.askhuman'));
  fs.writeFileSync(path.join(home, '.askhuman/config.json'), JSON.stringify({ general: {popupPrewarm:false, menuBarIcon:'off'}, channels: {popup:{enabled:false}} }));
  const globalFiles = ['.claude.json', '.claude/settings.json', '.codex/config.toml', '.codex/hooks.json', '.grok/config.toml', '.agents/mcp_config.json', '.pi/agent/extensions/askhuman/index.ts'];
  const globalSnapshot = new Map(globalFiles.map(file => [file, fs.existsSync(path.join(home,file)) ? fs.readFileSync(path.join(home,file)) : null]));
  const a = worktree('tree-a'), b = worktree('tree-b');
  fs.renameSync(system, `${system}.image`);
  fs.symlinkSync(`${system}.image`, system);
  assert.equal(JSON.parse(fs.readFileSync(path.join(a.root, '.askhuman-dev/workspace.json'))).systemBinary, system);
  assert.equal(json(['dev', 'target'], a.root).kind, 'system');
  for (const tree of [a, b]) {
    const target = json(['--instance', 'dev', '--worktree', tree.root, 'dev', 'target'], tree.root);
    assert.equal(target.kind, 'dev');
    assert.equal(target.home, path.join(tree.root, '.askhuman-dev/home'));
  }
  assert.equal(json(['dev', 'target'], b.root).kind, 'system');
  run(['--instance', 'system', 'dev', 'target'], a.root, { ASKHUMAN_HOME: path.join(a.root, '.askhuman-dev/home') }, false);
  assert.match(run(['--instance','dev','--worktree',b.root,'--output','text','must not ask the human'],temp,{},false).stderr, /dev ask/);
  const frames = await new Promise((resolve,reject)=>{
    const socket=createConnection(path.join(b.root,'.askhuman-dev/home/daemon.sock'));
    const rows=[];const timer=setTimeout(()=>{socket.destroy();reject(new Error('confirmation boundary timed out'));},5000);
    socket.on('error',reject);
    createInterface({input:socket}).on('line',line=>{
      const frame=JSON.parse(line);rows.push(frame);
      if (frame.type==='confirmFallback') {clearTimeout(timer);socket.destroy();resolve(rows);}
    });
    socket.once('connect',()=>socket.write(JSON.stringify({type:'submitConfirm',spec:{title:'automated isolation test',detail:{summary:'must not display'},choices:[{id:'allow',label:'Allow',role:'primary'},{id:'deny',label:'Deny',role:'destructive'}],presentation:{type:'singleSelectSubmit',submit_label:'Submit'},dismissActionId:'deny'},source:'fixture',lang:'en',project:b.root,agentKind:'codex',agentSessionId:'unbound-daily-session',callerPid:0})+'\n'));
  });
  assert.ok(frames.some(frame=>frame.type==='warn' && /isolated test (?:session binding|profile)/.test(frame.text)));
  assert.ok(frames.some(frame=>frame.type==='confirmFallback' && frame.reason==='invalidRequest'));
  assert.ok(!frames.some(frame=>frame.type==='confirmAccepted'));
  await new Promise((resolve,reject)=>{
    const socket=createConnection(path.join(b.root,'.askhuman-dev/home/daemon.sock'));
    const timer=setTimeout(()=>{socket.destroy();reject(new Error('native event boundary timed out'));},5000);
    socket.on('error',reject);socket.once('close',()=>{clearTimeout(timer);resolve();});
    socket.once('connect',()=>socket.end(JSON.stringify({type:'agentEvent',agent:'codex',event:'session-start',session_id:'unbound-daily-session',pid:process.pid,cwd:b.root,ts:0})+'\n'));
  });
  const agentsFile=path.join(b.root,'.askhuman-dev/home/agents.json');
  assert.doesNotMatch(fs.existsSync(agentsFile)?fs.readFileSync(agentsFile,'utf8'):'',/unbound-daily-session/);

  const pinned = json(['--instance', 'system', 'dev', 'target'], a.root);
  const inherited = { ASKHUMAN_RESOLVED_TARGET_V1: JSON.stringify(pinned) };
  assert.equal(json(['dev', 'target'], b.root, inherited).kind, 'system');
  run(['--instance', 'dev', '--worktree', a.root, 'dev', 'target'], a.root, inherited, false);
  run(['--instance', 'system', 'todo', 'add', 'formal-sentinel'], a.root);
  run(['--instance', 'dev', '--worktree', a.root, 'todo', 'add', 'dev-sentinel'], a.root);

  server = spawn(system, ['--instance', 'system', 'mcp'], { cwd: a.root, env, stdio: ['pipe', 'pipe', 'pipe'] });
  server.stderr.resume();
  const awaiting = new Map();
  function attachReplies(child) {
    createInterface({ input: child.stdout }).on('line', line => { const response = JSON.parse(line); if (response.id != null) { awaiting.get(response.id)?.(response); awaiting.delete(response.id); } });
  }
  attachReplies(server);
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
  assert.equal(catalog.tools.length, 6);
  assert.ok(!catalog.tools.some(tool => tool.name === 'dev_ask'));
  const todos = () => call('tools/call', { name: 'todo_list', arguments: {} });
  assert.match(JSON.stringify(await todos()), /formal-sentinel/);
  const manifestA = fs.readFileSync(path.join(a.root, '.askhuman-dev/workspace.json'));
  fs.unlinkSync(path.join(a.root, '.askhuman-dev/workspace.json'));
  assert.match(JSON.stringify(await todos()), /formal-sentinel/);
  assert.doesNotMatch(JSON.stringify(await todos()), /dev-sentinel/);
  fs.writeFileSync(path.join(a.root, '.askhuman-dev/workspace.json'), manifestA);

  const verificationId = randomUUID();
  const verificationDirectory = path.join(a.root, '.askhuman-dev/home/state/verifications');
  fs.mkdirSync(verificationDirectory, { recursive: true });
  fs.writeFileSync(path.join(verificationDirectory, `${verificationId}.json`), JSON.stringify({ version: 1, info: { id: verificationId, worktree: a.root, build: a.build, delivery: 'local', relay: false }, requestFingerprint: 'fixture', status: 'completed', executorPid: process.pid, stdout: '[user_input]\nverified-fixture-answer', exitCode: 0, createdAtMs: 0, updatedAtMs: 0 }));
  const dailyServer = server;
  server = spawn(process.execPath, [path.resolve('scripts/dev.mjs'), 'mcp', '--worktree', a.root], { cwd: temp, env, stdio: ['pipe','pipe','pipe'] });
  server.stderr.resume(); attachReplies(server);
  await call('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name:'project-smoke', version:'1' } });
  server.stdin.write(JSON.stringify({jsonrpc:'2.0',method:'notifications/initialized'})+'\n');
  assert.deepEqual((await call('tools/list', {})).tools.map(tool=>tool.name), ['dev_ask']);
  const answer = await call('tools/call', { name: 'dev_ask', arguments: { worktree: a.root, resume: verificationId } });
  assert.match(JSON.stringify(answer), /verified-fixture-answer/);
  assert.match(JSON.stringify(answer), /Dev verification metadata/);
  const changed = { ...a.build, binaryFingerprint: 'another-build' };
  fs.writeFileSync(path.join(verificationDirectory, `${verificationId}.json`), JSON.stringify({ version: 1, info: { id: verificationId, worktree: a.root, build: changed, delivery: 'local', relay: false }, requestFingerprint: 'fixture', status: 'completed', executorPid: process.pid, stdout: 'stale-answer', exitCode: 0, createdAtMs: 0, updatedAtMs: 0 }));
  assert.ok((await call('tools/call', { name: 'dev_ask', arguments: { worktree: a.root, resume: verificationId } })).isError);
  server.stdin.end(); server.kill(); server = dailyServer;
  assert.match(JSON.stringify(await todos()), /formal-sentinel/);
  assert.doesNotMatch(JSON.stringify(await todos()), /dev-sentinel/);
  server.stdin.end(); server.kill(); server = null;

  const manifestB = fs.readFileSync(path.join(b.root, '.askhuman-dev/workspace.json'));
  fs.unlinkSync(path.join(b.root, '.askhuman-dev/workspace.json'));
  run(['--instance', 'dev', '--worktree', b.root, 'dev', 'target'], temp, {}, false);
  fs.writeFileSync(path.join(b.root, '.askhuman-dev/workspace.json'), manifestB);
  fs.chmodSync(b.binary, 0o600);
  run(['--instance', 'dev', '--worktree', b.root, 'dev', 'target'], temp, {}, false);
  fs.chmodSync(b.binary, 0o700);
  fs.mkdirSync(path.join(b.root, 'src'));
  fs.writeFileSync(path.join(b.root, 'src/changed.rs'), '// unbuilt source');
  assert.match(run(['dev', 'ask', '--worktree', b.root, '--delivery', 'local', '--', 'must not show a question'], temp, {}, false).stderr, /source|rebuild/);
  fs.rmSync(path.join(b.root, 'src'), {recursive:true});
  const stub = `${b.binary}.old-fixture`;
  fs.writeFileSync(stub, '#!/bin/sh\nprintf old-client\n', {mode:0o700});
  fs.renameSync(stub,b.binary);
  recordBuild(b.root, b.binary, sourceFingerprint(b.root));
  assert.match(run(['dev', 'ask', '--worktree', b.root, '--delivery', 'local', '--', 'must not show a question'], temp, {}, false).stderr, /does not support/);
  const replacement = `${b.binary}.new-fixture`;
  fs.copyFileSync(original,replacement,fs.constants.COPYFILE_FICLONE);fs.chmodSync(replacement,0o700);
  fs.renameSync(replacement,b.binary);
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
    const rejected = run(['dev', 'ask', '--worktree', b.root, '--delivery', 'local', '--', '--invalid-routing-smoke-option'], a.root, {
      ASKHUMAN_RESOLVED_TARGET_V1: JSON.stringify(target), ASKHUMAN_HOME: target.home,
      ASKHUMAN_DEV_TEST_PROFILE: donorProfile.id,
    }, false);
    assert.doesNotMatch(rejected.stderr, /profile|test-profiles/);
    assert.match(rejected.stderr, /invalid-routing-smoke-option/);
    server = spawn(system, ['--instance', 'dev', '--worktree', a.root, 'mcp'], {
      cwd: donorProfile.neutralWorkspace, env: { ...env, ASKHUMAN_RESOLVED_TARGET_V1: JSON.stringify(target), ASKHUMAN_HOME: target.home, ASKHUMAN_DEV_TEST_PROFILE: donorProfile.id }, stdio: ['pipe','pipe','pipe'],
    });
    server.stderr.resume(); attachReplies(server);
    await call('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name:'private-profile-smoke',version:'1' } });
    server.stdin.write(JSON.stringify({jsonrpc:'2.0',method:'notifications/initialized'})+'\n');
    assert.match(JSON.stringify(await todos()), /dev-sentinel/);
    assert.doesNotMatch(JSON.stringify(await todos()), /formal-sentinel/);
    server.stdin.end();server.kill();server=null;

  }
  for (const [file, bytes] of globalBefore) assert.deepEqual(fs.existsSync(path.join(home, file)) ? fs.readFileSync(path.join(home, file)) : null, bytes);

  for (const [file, bytes] of globalSnapshot) assert.deepEqual(fs.existsSync(path.join(home,file)) ? fs.readFileSync(path.join(home,file)) : null, bytes, `project prep must preserve ${file}`);
  assert.equal(fs.existsSync(path.join(home, '.askhuman/development/routing.json')), false);
  assert.match(run(['dev','ask','--worktree',a.root,'--','must not show a question'],temp,{},false).stderr, /formal relay|formal daemon|remote channel/);
  const legacy = path.join(b.root, '.askhuman-dev/enabled');
  fs.writeFileSync(legacy, 'legacy marker');
  assert.equal(json(['dev','target'], b.root).kind, 'dev');
  run(['dev','enable','--agent','codex'], b.root);
  assert.equal(fs.existsSync(legacy), false);
  assert.equal(fs.readFileSync(path.join(b.root, '.askhuman-dev/legacy-enabled.before'),'utf8'), 'legacy marker');
  assert.equal(json(['dev','target'], b.root).kind, 'system');
  console.log('PASS: project-only prep; two worktrees; normal six tools plus project dev_ask; system/Dev/system; resume; stale builds; native profile isolation; legacy marker upgrade; globals preserved');

} finally {
  server?.kill();
  for (const instanceHome of homes) {
    spawnSync(system, ['daemon','stop','--force'], {cwd:temp,env:{...env,ASKHUMAN_HOME:instanceHome},encoding:'utf8',timeout:10000});
    if (process.platform === 'darwin') {
      const suffix = createHash('sha256').update(instanceHome).digest('hex').slice(0, 16);
      spawnSync('launchctl', ['bootout', `gui/${process.getuid()}/com.naituw.humaninloop.daemon.instance.${suffix}`], {stdio:'ignore',timeout:10000});
    }
  }
  for (const workspace of workspaces) {
    const owner = JSON.parse(fs.readFileSync(path.join(workspace, "askhuman-owner.json")));
    if (owner.worktree.startsWith(temp + path.sep)) fs.rmSync(workspace, { recursive: true, force: true });
  }
  fs.rmSync(temp, { recursive: true, force: true });
}
