// Private real-process routing and old-build handoff regressions. No GUI, real bot, or model.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import http from "node:http";
import { spawn, spawnSync } from "node:child_process";
import { createConnection } from "node:net";
import { createInterface } from "node:readline";

if (process.platform === "win32") {
  console.log("SKIP: run the native Windows fixture and Known Folder matrix");
  process.exit(0);
}
const candidate = path.resolve(process.argv[2] ?? "src-tauri/target/local-install/AskHuman");
const old = process.argv[3] && path.resolve(process.argv[3]);
const root = fs.realpathSync(fs.mkdtempSync(path.join(process.platform === "darwin" ? "/tmp" : os.tmpdir(), "ah-home-")));
const clean = { ...process.env };
for (const key of Object.keys(clean)) {
  if (/^(ASKHUMAN_|CODEX_|CLAUDE_|CURSOR_|GROK_|PI_SESSION|PI_CODING)/.test(key)) delete clean[key];
}
Object.assign(clean, { ASKHUMAN_NO_KEYCHAIN: "1", GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: path.join(root, "gitconfig") });
fs.writeFileSync(clean.GIT_CONFIG_GLOBAL, "");
const fixtureFlag = "--internal-system-fixture-v2";
const ack = "ASKHUMAN_SYSTEM_FIXTURE_V2_ACTIVE";
const children = [], calls = [], cards = [], updates = [];
let nextUpdate = 0, nextMessage = 0;
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const api = http.createServer(async (request, response) => {
  let raw = ""; for await (const part of request) raw += part;
  const body = raw ? JSON.parse(raw) : {}, method = request.url.split("/").at(-1);
  let result = true;
  if (method === "getUpdates") { await pause(30); result = updates.filter(update => update.update_id >= (body.offset ?? 0)); }
  if (method === "sendMessage") {
    const id = ++nextMessage;
    result = { message_id: id, chat: { id: 1 }, text: body.text };
    if (body.reply_markup?.inline_keyboard) cards.push({ id, text: body.text });
  }
  response.setHeader("content-type", "application/json"); response.end(JSON.stringify({ ok: true, result }));
});
await new Promise(resolve => api.listen(0, "127.0.0.1", resolve));

function copy(from, to) {
  fs.mkdirSync(path.dirname(to), { recursive: true });
  fs.copyFileSync(from, to, fs.constants.COPYFILE_FICLONE); fs.chmodSync(to, 0o700);
}
function setup(name, source = candidate) {
  const directory = path.join(root, name), account = path.join(directory, "user"), data = path.join(account, ".askhuman"), binary = path.join(directory, "bin/AskHuman");
  fs.mkdirSync(data, { recursive: true }); copy(source, binary);
  fs.writeFileSync(path.join(data, "config.json"), JSON.stringify({
    general: { menuBarIcon: "off", popupPrewarm: false, daemonLifecycle: "activity" },
    channels: { autoActivation: false, popup: { enabled: false }, telegram: { enabled: true, botToken: "123:fixture", chatId: "1", apiBaseUrl: `http://127.0.0.1:${api.address().port}` } },
  }));
  return { directory, account, data, binary, env: { ...clean, HOME: account } };
}
function protectedArgs(instance, args) { return ["--development-capabilities", fixtureFlag, instance.account, "--", ...args]; }
function sync(program, args, env = clean, success = true) {
  const result = spawnSync(program, args, { cwd: root, env, encoding: "utf8", timeout: 15000 });
  if (success) assert.equal(result.status, 0, result.stderr || result.error?.message);
  return result;
}
function start(instance, args, env = instance.env) {
  const log = fs.openSync(path.join(instance.directory, "daemon.log"), "a");
  const child = spawn(instance.binary, args, { cwd: root, env, stdio: ["ignore", log, log] });
  fs.closeSync(log); children.push(child); return child;
}
function ask(instance, text, args, env = instance.env) {
  const child = spawn(instance.binary, args ?? protectedArgs(instance, ["-q", text, "-o", "ok"]), { cwd: root, env, stdio: ["ignore", "pipe", "pipe"] });
  calls.push(child); let stdout = "", stderr = "";
  child.stdout.on("data", data => stdout += data); child.stderr.on("data", data => stderr += data);
  const finished = new Promise(resolve => child.once("close", code => resolve({ code, stdout, stderr })));
  return { child, finished };
}
function frame(instance, message) {
  return new Promise((resolve, reject) => {
    const socket = createConnection(path.join(instance.data, "daemon.sock"));
    const timer = setTimeout(() => { socket.destroy(); reject(new Error("fixture IPC timed out")); }, 3000);
    const finish = (error, value) => { clearTimeout(timer); socket.destroy(); error ? reject(error) : resolve(value); };
    socket.on("error", error => finish(error));
    createInterface({ input: socket }).on("error", () => {}).on("line", line => finish(null, JSON.parse(line)));
    socket.on("connect", () => socket.write(JSON.stringify(message) + "\n"));
  });
}
async function status(instance) { const result = await frame(instance, { type: "status" }); assert.equal(result.type, "status"); return result.info ?? result; }
async function until(fn, label) {
  const deadline = Date.now() + 90000;
  while (Date.now() < deadline) { try { if (await fn()) return; } catch {} await pause(100); }
  throw new Error(`${label} timed out; evidence ${root}`);
}
function answer(card) {
  for (const data of ["toggle:0", "submit"]) updates.push({ update_id: ++nextUpdate, callback_query: { id: String(nextUpdate), data, from: { id: 1 }, message: { message_id: card.id, chat: { id: 1 } } } });
}
function replace(instance, source, revision) {
  const next = instance.binary + ".new"; copy(source, next);
  if (revision) {
    if (process.platform === "darwin") sync("/usr/bin/codesign", ["--force", "--sign", "-", "--identifier", `com.askhuman.home-fixture.${revision}`, next]);
    else fs.appendFileSync(next, `\nfixture revision ${revision}\n`);
  }
  fs.renameSync(next, instance.binary);
}

try {
  assert.equal(JSON.parse(sync(candidate, ["--development-capabilities"]).stdout).systemFixture, 2);
  const poison = path.join(root, "caller-home"); fs.mkdirSync(poison);
  const expected = path.join(os.userInfo().homedir, ".askhuman");
  for (const HOME of [poison, "", undefined]) {
    const environment = { ...clean, HOME };
    if (HOME === undefined) delete environment.HOME;
    for (const args of [["dev", "target"], ["--instance", "system", "dev", "target"]]) {
      const result = JSON.parse(sync(candidate, args, environment).stdout);
      assert.equal(result.kind, "system"); assert.equal(result.home, expected);
    }
  }
  assert.ok(!fs.existsSync(path.join(poison, ".askhuman")));
  console.log("PASS: ordinary and explicit System ignore temporary or empty HOME");
  const oldHelper = sync(candidate, ["todo-sync", "tick", "--reason", "old-helper"], {
    ...clean, HOME: poison, ASKHUMAN_TODO_SYNC_CALLER: "helper",
    ASKHUMAN_TODO_SYNC_DIR: path.join(poison, ".askhuman/state/todo-sync"),
  }, false);
  assert.notEqual(oldHelper.status, 0); assert.match(oldHelper.stderr, /old helper.*temporary HOME/);
  assert.ok(!fs.existsSync(path.join(poison, ".askhuman")));
  console.log("PASS: an already-running HOME-only helper cannot tick the real account after upgrade");

  const fixture = setup("fixture");
  const poisoned = { ...fixture.env, HOME: poison };
  const first = ask(fixture, "fixture cold start", undefined, poisoned);
  await until(() => cards.some(card => /fixture cold start/.test(card.text)), "private cold-start question");
  const ready = await status(fixture);
  assert.equal(ready.systemFixture.accountHome, fixture.account);
  assert.equal(ready.socket, path.join(fixture.data, "daemon.sock"));
  answer(cards.find(card => /fixture cold start/.test(card.text)));
  const result = await first.finished;
  assert.equal(result.code, 0, result.stderr); const activation = JSON.parse(result.stderr.split("\n").find(line => line.startsWith(ack + " ")).slice(ack.length + 1));
  assert.deepEqual(activation, { version: 2, accountHome: fixture.account, kind: "system", home: fixture.data }); assert.match(result.stdout, /ok/);
  assert.ok(!fs.existsSync(path.join(poison, ".askhuman")));
  console.log("PASS: poisoned HOME cold-starts only the explicit private System and returns its answer");

  const badAccount = path.join(root, "linked-account"); fs.mkdirSync(badAccount);
  fs.symlinkSync(expected, path.join(badAccount, ".askhuman"));
  const rejected = sync(candidate, ["--development-capabilities", fixtureFlag, badAccount, "--", "dev", "target"], clean, false);
  assert.notEqual(rejected.status, 0); assert.match(rejected.stderr, /redirection|conflicts/);
  const deniedManagement = sync(candidate, protectedArgs(fixture, ["dev", "target"]), { ...clean, ASKHUMAN_HOME: expected }, false);
  assert.notEqual(deniedManagement.status, 0); assert.match(deniedManagement.stderr, /conflicts with production/);
  assert.ok(!deniedManagement.stderr.includes(ack));
  const managementAlias = path.join(root, "management-alias"); fs.symlinkSync(expected, managementAlias);
  const deniedDescendant = sync(candidate, protectedArgs(fixture, ["dev", "target"]), { ...clean, ASKHUMAN_HOME: path.join(managementAlias, "fixture-must-not-create") }, false);
  assert.notEqual(deniedDescendant.status, 0); assert.match(deniedDescendant.stderr, /conflicts with production/);
  assert.ok(!fs.existsSync(path.join(expected, "fixture-must-not-create")));
  console.log("PASS: fixture data-root aliases are rejected before business dispatch");

  if (old) {
    const capabilities = JSON.parse(sync(old, ["--development-capabilities"]).stdout);
    assert.equal(capabilities.developmentVerification, 2); assert.equal(capabilities.systemFixture, undefined);
    const guardedOld = sync(old, protectedArgs(fixture, ["config", "set", "general.language", "en"]), clean);
    assert.ok(!guardedOld.stderr.includes(ack));
    const envelope = { version: 2, kind: "system", binary: old, home: fixture.data, worktree: null, reason: "fixture", noKeychain: true, fixture: { version: 2, accountHome: fixture.account } };
    assert.notEqual(sync(old, ["dev", "target"], { ...clean, ASKHUMAN_RESOLVED_TARGET_V1: JSON.stringify(envelope) }, false).status, 0);
    console.log("PASS: real old Dev-v2 binary exits safely at the guarded entry and rejects inherited fixture v2");

    const legacy = setup("legacy", old);
    start(legacy, ["daemon", "run"]);
    await until(() => status(legacy), "old temporary System startup");
    const initial = await status(legacy);
    const pinned = JSON.parse(sync(legacy.binary, ["dev", "target"], legacy.env).stdout);
    const pending = ask(legacy, "legacy pending A", ["-q", "legacy pending A", "-o", "ok"]);
    await until(() => cards.some(card => /legacy pending A/.test(card.text)), "old pending card");
    replace(legacy, candidate);
    await until(async () => { const current = await status(legacy); return current.fingerprint.hash !== initial.fingerprint.hash && !current.draining; }, "old-to-new original-root handoff");
    const continued = await status(legacy);
    assert.equal(continued.pid, initial.pid); assert.equal(continued.socket, initial.socket);
    assert.equal(JSON.parse(sync(legacy.binary, ["dev", "target"], legacy.env).stdout).home, expected);
    const inheritedEnv = { ...legacy.env, ASKHUMAN_RESOLVED_TARGET_V1: JSON.stringify(pinned) };
    const denied = sync(legacy.binary, ["config", "unset", "channels.telegram.botToken"], inheritedEnv, false);
    assert.notEqual(denied.status, 0); assert.match(denied.stderr, /read-only/);
    for (const op of [{ op: "revoke", phoneId: "fixture-unknown-phone" }, { op: "rename", phoneId: "fixture-unknown-phone", name: "blocked" }]) {
      const reply = await frame(legacy, { type: "ios", op });
      assert.equal(reply.type, "error"); assert.match(reply.message, /read-only/);
    }
    answer(cards.find(card => /legacy pending A/.test(card.text)));
    const completed = await pending.finished;
    assert.equal(completed.code, 0, completed.stderr); assert.match(completed.stdout, /ok/);
    const next = ask(legacy, "legacy new B", ["-q", "legacy new B", "-o", "ok"], inheritedEnv);
    await until(() => cards.some(card => /legacy new B/.test(card.text)), "continued legacy new card");
    answer(cards.find(card => /legacy new B/.test(card.text)));
    assert.equal((await next.finished).code, 0);
    console.log("PASS: old temporary System retains its PID, socket, pending answer and new requests through exec; new ordinary callers choose the real account");

    replace(fixture, old);
    await pause(17000);
    assert.equal((await status(fixture)).pid, ready.pid);
    assert.equal((await status(fixture)).fingerprint.hash, ready.fingerprint.hash);
    replace(fixture, candidate, 1);
    await until(async () => (await status(fixture)).fingerprint.hash !== ready.fingerprint.hash, "compatible fixture replacement");
    assert.equal((await status(fixture)).systemFixture.accountHome, fixture.account);
    console.log("PASS: fixture service refuses an old replacement and accepts a compatible replacement");
  }
  console.log(`Evidence ${root}`);
} finally {
  for (const call of calls) if (call.exitCode === null) call.kill();
  for (const directory of fs.readdirSync(root)) {
    const data = path.join(root, directory, "user/.askhuman");
    if (fs.existsSync(path.join(data, "daemon.sock"))) {
      try { await frame({ data }, { type: "stop", force: true }); } catch {}
    }
  }
  for (const child of children) if (child.exitCode === null) child.kill();
  api.closeAllConnections?.(); api.close();
}
