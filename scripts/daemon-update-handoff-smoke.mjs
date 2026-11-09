// Runs private real daemons and a loopback Telegram stand-in. No Cursor, model, GUI, or real bot.
// It deliberately polls Status without Hello: a client must not be what restarts the daemon.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { spawn, execFileSync } from 'node:child_process';
import { createConnection } from 'node:net';
import { createInterface } from 'node:readline';
import { randomUUID } from 'node:crypto';
if (process.platform === 'win32') throw new Error('Use native Windows handoff verification; this fixture uses Unix sockets');
const source = path.resolve(process.argv[2] ?? 'src-tauri/target/local-install/AskHuman');
const root = fs.realpathSync(fs.mkdtempSync(path.join(process.platform === 'darwin' ? '/tmp' : os.tmpdir(), 'ah-handoff-')));
const binary = path.join(root, 'AskHuman'), home = path.join(root, 'user'), ah = path.join(root, 'ah');
fs.mkdirSync(home); fs.mkdirSync(ah); fs.copyFileSync(source, binary); fs.chmodSync(binary, 0o700);
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(ASKHUMAN_|CODEX_|CLAUDE_|CURSOR_|GROK_|PI_SESSION|PI_CODING)/.test(key)) delete env[key];
Object.assign(env, { HOME: home, ASKHUMAN_HOME: ah, ASKHUMAN_NO_KEYCHAIN: '1', GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: path.join(root, 'gitconfig') });
fs.writeFileSync(env.GIT_CONFIG_GLOBAL, '');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(fn, name) {
  const deadline = Date.now() + 90000;
  while (Date.now() < deadline) { try { if (await fn()) return; } catch {} await sleep(100); }
  throw new Error(`${name} timed out; logs: ${root}`);
}
let update = 0, message = 0, revision = 0;
const updates = [], cards = [], acknowledgements = new Set(), offsets = [];
const api = http.createServer(async (req, res) => {
  let raw = ''; for await (const part of req) raw += part;
  const body = raw ? JSON.parse(raw) : {}, method = req.url.split('/').at(-1);
  let result = true;
  if (method === 'getUpdates') {
    offsets.push(body.offset ?? 0);
    await sleep(30);
    while (updates.length && updates[0].update_id < (body.offset ?? 0)) updates.shift();
    result = updates.slice();
  }
  if (method === 'answerCallbackQuery') acknowledgements.add(body.callback_query_id);
  if (method === 'sendMessage') {
    const id = ++message; result = { message_id: id, chat: { id: 1 }, text: body.text };
    if (body.reply_markup?.inline_keyboard) cards.push({ id, keyboard: body.reply_markup.inline_keyboard });
  }
  res.setHeader('content-type', 'application/json'); res.end(JSON.stringify({ ok: true, result }));
});
await new Promise(resolve => api.listen(0, '127.0.0.1', resolve));
fs.writeFileSync(path.join(ah, 'config.json'), JSON.stringify({ general: { menuBarIcon: 'off', popupPrewarm: false, daemonLifecycle: 'keepalive' }, channels: { autoActivation: false, popup: { enabled: false }, telegram: { enabled: true, botToken: '123:fixture', chatId: '1', apiBaseUrl: `http://127.0.0.1:${api.address().port}` } } }));
function ipc(msg) {
  return new Promise((resolve, reject) => {
    const socket = createConnection(path.join(ah, 'daemon.sock'));
    const timer = setTimeout(() => { socket.destroy(); reject(new Error('IPC timed out')); }, 3000);
    const done = (err, value) => { clearTimeout(timer); socket.destroy(); err ? reject(err) : resolve(value); };
    socket.on('error', err => done(err));
    createInterface({ input: socket }).on('error', () => {}).on('line', line => done(null, JSON.parse(line)));
    socket.on('connect', () => socket.write(JSON.stringify(msg) + '\n'));
  });
}
const status = () => ipc({ type: 'status' });
let daemon, permissionFixture, legacyFixture;
const callers = [];
function start(extra = {}) {
  const log = fs.openSync(path.join(root, 'daemon.log'), 'a');
  daemon = spawn(binary, ['daemon', 'run'], { env: { ...env, ...extra }, cwd: root, stdio: ['ignore', log, log] });
  fs.closeSync(log); return daemon;
}
function replace() {
  const next = binary + '.new'; fs.copyFileSync(source, next); fs.chmodSync(next, 0o700);
  if (process.platform === 'darwin') execFileSync('/usr/bin/codesign', ['--force', '--sign', '-', '--identifier', `com.askhuman.fixture.${++revision}`, next], { stdio: 'ignore' });
  else fs.appendFileSync(next, `\nfixture revision ${++revision}\n`);
  fs.renameSync(next, binary);
}
function ask(text = 'private update fixture') {
  const child = spawn(binary, ['-q', text, '-o', 'ok'], { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] });
  callers.push(child);
  let stdout = '', stderr = '';
  child.stdout.on('data', b => stdout += b); child.stderr.on('data', b => stderr += b);
  const finished = new Promise(resolve => child.on('exit', code => resolve({ code, stdout, stderr })));
  return { child, finished };
}
function callback(id, data) {
  const callbackId = String(++update);
  updates.push({ update_id: update, callback_query: { id: callbackId, data, from: { id: 1 }, message: { message_id: id, chat: { id: 1 } } } });
  return callbackId;
}
function answer(id) {
  callback(id, 'toggle:0');
  callback(id, 'submit');
}
function legacyAsk() {
  let socket;
  const finished = new Promise((resolve, reject) => {
    socket = createConnection(path.join(ah, 'daemon.sock'));
    socket.on('error', reject);
    createInterface({ input: socket }).on('error', reject).on('line', line => {
      const message = JSON.parse(line);
      if (message.type === 'final') resolve({ code: message.exit_code, stdout: message.stdout });
    });
    socket.on('connect', () => socket.write(JSON.stringify({
      type: 'submit', message: { text: '' }, questions: [{ message: 'private legacy fixture', predefinedOptions: ['ok'] }],
      isMarkdown: false, source: 'fixture', lang: 'en', project: root, callerPid: process.pid, recordHistory: false,
    }) + '\n'));
  });
  return { finished, cancel: () => socket?.destroy() };
}
function confirm() {
  const call = { id: randomUUID(), secret: randomUUID(), version: 1, pid: process.pid };
  const context = ['agent', 'project', 'workspace', 'tool', 'permission_mode', 'created_at'].map(id => ({ id, label: id, value: 'fixture', kind: 'text' }));
  const task = {
    spec: {
      title: 'private permission fixture', context, detail: { summary: 'Allow a fixture operation', bodyMd: '' },
      choices: [{ id: 'approve_once', label: 'Approve once', description: '', role: 'primary' }, { id: 'deny', label: 'Deny', description: '', role: 'destructive' }],
      presentation: { type: 'singleSelectSubmit', submit_label: 'Submit' }, dismissActionId: 'deny',
    },
    source: 'fixture', lang: 'en', project: root, agentKind: 'claude', agentSessionId: 'fixture-confirm',
  };
  let socket;
  let accepted = false;
  const finished = new Promise((resolve, reject) => {
    const connect = () => {
      socket = createConnection(path.join(ah, 'daemon.sock'));
      socket.on('error', () => {});
      createInterface({ input: socket }).on('error', () => {}).on('line', line => {
        const message = JSON.parse(line);
        if (message.type !== 'recovery') return;
        if (message.event.event === 'accepted') accepted = true;
        if (message.event.event === 'final') {
          socket.removeAllListeners('close');
          socket.destroy();
          resolve(message.event.outcome);
        }
      });
      socket.on('connect', () => socket.write(JSON.stringify({ type: 'recover', version: 1, operation: { op: 'submit', call, task: { kind: 'confirm', task } } }) + '\n'));
      socket.on('close', () => setTimeout(connect, 100));
    };
    connect();
    setTimeout(() => reject(new Error(`permission fixture timed out, accepted=${accepted}`)), 120000).unref();
  });
  return { finished, cancel: () => { socket?.removeAllListeners('close'); socket?.destroy(); } };
}
const differs = (a, b) => a?.hash !== b?.hash || a?.size !== b?.size;
async function stopped(child) { await until(() => child.exitCode !== null || child.signalCode !== null, 'old process exit'); }
try {
  start(); await until(status, 'startup');
  const first = await status(); replace();
  await until(async () => { const s = await status(); return !s.draining && differs(s.fingerprint, first.fingerprint); }, 'unassisted update handoff');
  assert.equal((await status()).pid, first.pid, 'Unix exec keeps its PID');
  console.log('PASS: no tray or waiting CLI is needed for update handoff');

  const ordinary = ask('pending A');
  await until(() => cards.length === 1, 'ordinary question');
  const originalCard = cards[0].id;
  const selected = callback(originalCard, 'toggle:0');
  await until(() => acknowledgements.has(selected), 'saved partial selection');
  const before = await status();
  const pendingUpdateAt = Date.now();
  replace();
  await until(async () => { const s = await status(); return !s.draining && differs(s.fingerprint, before.fingerprint); }, 'handoff with unanswered A');
  assert.equal(ordinary.child.exitCode, null);
  assert.equal(cards.length, 1, 'restoration must reuse the original card');
  assert.ok(offsets.at(-1) >= Number(selected) + 1, 'Telegram consumption resumes at its saved offset');
  const updateReadyMs = Date.now() - pendingUpdateAt;
  const newQuestionAt = Date.now();
  const newer = ask('new B');
  await until(() => cards.length === 2, 'new B while A remains unanswered');
  const newQuestionMs = Date.now() - newQuestionAt;
  answer(cards[1].id);
  assert.equal((await newer.finished).code, 0);
  assert.equal(ordinary.child.exitCode, null);
  callback(originalCard, 'submit');
  const result = await ordinary.finished;
  assert.equal(result.code, 0);
  assert.match(result.stdout, /ok/);
  assert.equal((result.stdout.match(/\[selected_options\]/g) ?? []).length, 1, 'one CLI result is printed');
  console.log('PASS: pending A survives an update, new B arrives, and A keeps its selected option and original card');
  console.log(`TIMING: update detected and new daemon visible in ${updateReadyMs} ms; new B card arrived in ${newQuestionMs} ms`);

  const permission = permissionFixture = confirm();
  await until(() => cards.length === 3, 'permission confirmation');
  const permissionCard = cards[2].id;
  const choice = callback(permissionCard, 'pc:do:0');
  await until(() => acknowledgements.has(choice), 'saved permission selection');
  const permissionVersion = await status(); replace();
  await until(async () => { const s = await status(); return !s.draining && differs(s.fingerprint, permissionVersion.fingerprint); }, 'handoff with pending confirmation');
  assert.equal(cards.length, 3);
  callback(permissionCard, 'pc:submit');
  assert.equal((await permission.finished).result.actionId, 'approve_once');
  permission.cancel();
  console.log('PASS: permission confirmation keeps its original card and selection across handoff');

  const legacy = legacyFixture = legacyAsk();
  await until(() => cards.length === 4, 'legacy request');
  const legacyVersion = await status(); replace();
  await sleep(17000);
  const held = await status();
  assert.equal(held.draining, false);
  assert.equal(differs(held.fingerprint, legacyVersion.fingerprint), false);
  const compatible = ask('compatible question alongside a legacy caller');
  await until(() => cards.length === 5, 'compatible request while legacy question is waiting');
  answer(cards[4].id);
  assert.equal((await compatible.finished).code, 0);
  answer(cards[3].id);
  assert.equal((await legacy.finished).code, 0);
  legacy.cancel();
  await until(async () => { const s = await status(); return !s.draining && differs(s.fingerprint, legacyVersion.fingerprint); }, 'handoff after legacy caller finishes');
  console.log('PASS: a legacy caller delays replacement while compatible new questions continue arriving');

  const question = ask('explicit stop fixture');
  await until(() => cards.length === 6, 'question before explicit stop');
  await ipc({ type: 'stop', force: false });
  replace();
  answer(cards[5].id);
  assert.equal((await question.finished).code, 0);
  await stopped(daemon); await sleep(1500);
  await assert.rejects(status);
  console.log('PASS: explicit stop overrides a pending update');

  start({ ASKHUMAN_DAEMON_AUTORESTART: '0' }); await until(status, 'disabled autorestart startup');
  const disabled = await status(); replace(); await sleep(17000);
  const same = await status(); assert.equal(same.draining, false); assert.equal(differs(same.fingerprint, disabled.fingerprint), false);
  await ipc({ type: 'stop', force: true }); await stopped(daemon);
  console.log('PASS: autorestart=0 is respected');

  start(); await until(status, 'exec failure scenario startup');
  const beforeExecFailure = await status();
  const capability = execFileSync(source, ['--recovery-capabilities'], { encoding: 'utf8' }).trim();
  const failingExec = `#!/bin/sh\nif [ "$1" = "--recovery-capabilities" ]; then\n  printf '%s\\n' '${capability}'\n  chmod 600 "$0"\n  exit 0\nfi\nexit 1\n`;
  fs.writeFileSync(binary + '.exec-failure', failingExec);
  fs.chmodSync(binary + '.exec-failure', 0o700);
  fs.renameSync(binary + '.exec-failure', binary);
  await until(() => fs.readFileSync(path.join(root, 'daemon.log'), 'utf8').includes('recovering requests in the current daemon'), 'exec failure recovery');
  await until(status, 'old daemon recovered after failed exec');
  const recovered = await status();
  assert.deepEqual(recovered.fingerprint, beforeExecFailure.fingerprint, 'failed exec must keep the running image identity');
  replace();
  await until(async () => { const s = await status(); return !s.draining && differs(s.fingerprint, beforeExecFailure.fingerprint); }, 'retry after failed exec');
  await ipc({ type: 'stop', force: true }); await stopped(daemon);
  console.log('PASS: failed exec preserves the old running fingerprint and a repaired replacement is retried');

  start(); await until(status, 'failure scenario startup');
  const healthy = await status();
  fs.writeFileSync(binary + '.broken', 'not an executable'); fs.chmodSync(binary + '.broken', 0o700); fs.renameSync(binary + '.broken', binary);
  await sleep(17000);
  assert.equal(daemon.exitCode, null);
  assert.equal((await status()).draining, false, 'a failed compatibility probe must keep service available');
  replace();
  await until(async () => { const s = await status(); return !s.draining && differs(s.fingerprint, healthy.fingerprint); }, 'automatic retry after replacement repair');
  await ipc({ type: 'stop', force: true }); await stopped(daemon);
  console.log('PASS: an invalid replacement keeps the old service running and a repaired replacement is retried');
  console.log(`Logs: ${root}`);
} finally {
  permissionFixture?.cancel();
  legacyFixture?.cancel();
  for (const caller of callers) {
    if (caller.exitCode === null && caller.signalCode === null) caller.kill('SIGTERM');
  }
  if (daemon && daemon.exitCode === null && daemon.signalCode === null) { try { await ipc({ type: 'stop', force: true }); } catch {} daemon.kill('SIGTERM'); }
  api.closeAllConnections();
  await new Promise(resolve => api.close(resolve));
}
