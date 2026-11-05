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
  const deadline = Date.now() + 45000;
  while (Date.now() < deadline) { try { if (await fn()) return; } catch {} await sleep(100); }
  throw new Error(`${name} timed out; logs: ${root}`);
}
let update = 0, message = 0, revision = 0;
const updates = [], cards = [];
const api = http.createServer(async (req, res) => {
  let raw = ''; for await (const part of req) raw += part;
  const body = raw ? JSON.parse(raw) : {}, method = req.url.split('/').at(-1);
  let result = true;
  if (method === 'getUpdates') { await sleep(30); result = updates.splice(0); }
  if (method === 'sendMessage') {
    const id = ++message; result = { message_id: id, chat: { id: 1 }, text: body.text };
    if (body.reply_markup?.inline_keyboard?.flat().some(b => b.callback_data === 'submit')) cards.push(id);
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
let daemon;
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
function ask() {
  const child = spawn(binary, ['-q', 'private update fixture', '-o', 'ok'], { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '', stderr = '';
  child.stdout.on('data', b => stdout += b); child.stderr.on('data', b => stderr += b);
  const finished = new Promise(resolve => child.on('exit', code => resolve({ code, stdout, stderr })));
  return { child, finished };
}
function answer(id) {
  for (const data of ['toggle:0', 'submit']) updates.push({ update_id: ++update, callback_query: { id: String(update), data, from: { id: 1 }, message: { message_id: id, chat: { id: 1 } } } });
}
const differs = (a, b) => a?.hash !== b?.hash || a?.size !== b?.size;
async function stopped(child) { await until(() => child.exitCode !== null || child.signalCode !== null, 'old process exit'); }
try {
  start(); await until(status, 'startup');
  const first = await status(); replace();
  await until(async () => { const s = await status(); return !s.draining && differs(s.fingerprint, first.fingerprint); }, 'unassisted update handoff');
  assert.equal((await status()).pid, first.pid, 'Unix exec keeps its PID');
  console.log('PASS: no tray or waiting CLI is needed for update handoff');

  const ordinary = ask(); await until(() => cards.length >= 1, 'ordinary question');
  const before = await status(); replace();
  await until(async () => (await status()).draining, 'drain while waiting');
  assert.equal(ordinary.child.exitCode, null);
  answer(cards[0]); assert.equal((await ordinary.finished).code, 0);
  await until(async () => { const s = await status(); return !s.draining && differs(s.fingerprint, before.fingerprint); }, 'handoff after last answer');
  console.log('PASS: an existing question finishes before automatic handoff');

  const question = ask(); await until(() => cards.length >= 2, 'second ordinary question');
  replace(); await until(async () => (await status()).draining, 'second drain');
  await ipc({ type: 'stop', force: false });
  answer(cards[1]); assert.equal((await question.finished).code, 0);
  await stopped(daemon); await sleep(1500);
  await assert.rejects(status);
  console.log('PASS: explicit stop overrides a pending update');

  start({ ASKHUMAN_DAEMON_AUTORESTART: '0' }); await until(status, 'disabled autorestart startup');
  const disabled = await status(); replace(); await sleep(17000);
  const same = await status(); assert.equal(same.draining, false); assert.equal(differs(same.fingerprint, disabled.fingerprint), false);
  await ipc({ type: 'stop', force: true }); await stopped(daemon);
  console.log('PASS: autorestart=0 is respected');

  start(); await until(status, 'failure scenario startup');
  fs.writeFileSync(binary + '.broken', 'not an executable'); fs.chmodSync(binary + '.broken', 0o700); fs.renameSync(binary + '.broken', binary);
  await stopped(daemon);
  const failure = JSON.parse(fs.readFileSync(path.join(ah, 'daemon-handoff.json'), 'utf8'));
  assert.equal(failure.failed, true);
  console.log('PASS: an invalid replacement fails finitely and leaves a diagnostic');
  console.log(`Logs: ${root}`);
} finally {
  if (daemon && daemon.exitCode === null && daemon.signalCode === null) { try { await ipc({ type: 'stop', force: true }); } catch {} daemon.kill('SIGTERM'); }
  await new Promise(resolve => api.close(resolve));
}
