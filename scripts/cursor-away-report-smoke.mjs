// Exercises real CLI/MCP/daemon flows against a private account fixture and loopback Telegram.
// All Cursor identities and binding tokens below belong to this fixture.
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { createConnection } from "node:net";
import { createInterface } from "node:readline";

if (process.platform === "win32") {
  console.log("SKIP private account smoke requires Unix sockets");
  process.exit(0);
}
const source = path.resolve(process.argv[2] ?? "src-tauri/target/local-install/AskHuman");
const root = fs.realpathSync(fs.mkdtempSync(path.join(process.platform === "darwin" ? "/tmp" : os.tmpdir(), "ah-cursor-report-")));
const account = path.join(root, "account"), data = path.join(account, ".askhuman");
fs.mkdirSync(data, { recursive: true });
const binary = path.join(root, "AskHuman");
fs.copyFileSync(source, binary); fs.chmodSync(binary, 0o700);
const prefix = ["--development-capabilities", "--internal-system-fixture-v2", account, "--"];
const env = { ...process.env };
for (const key of Object.keys(env)) {
  if (/^(ASKHUMAN_|CODEX_|CLAUDE_|CURSOR_|GROK_|PI_)/.test(key)) delete env[key];
}
Object.assign(env, {
  HOME: path.join(root, "caller"), ASKHUMAN_NO_KEYCHAIN: "1",
  GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: path.join(root, "gitconfig"),
});
fs.writeFileSync(env.GIT_CONFIG_GLOBAL, "");
const cursor = { ...env, ASKHUMAN_FROM_MCP: "1", ASKHUMAN_MCP_INSTANCE_ID: "private-cursor", ASKHUMAN_MCP_AGENT_KIND: "cursor", ASKHUMAN_MCP_AGENT_SESSION_ID: "private-session" };
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const children = new Set(), cards = [], updates = [];
let update = 0, message = 0, autoAnswer = true, daemon;
function answer(card) {
  for (const callbackData of ["toggle:0", "submit"]) {
    const id = ++update;
    updates.push({ update_id: id, callback_query: { id: String(id), data: callbackData, from: { id: 1 }, message: { message_id: card.id, chat: { id: 1 } } } });
  }
}
const api = http.createServer(async (request, response) => {
  let raw = ""; for await (const chunk of request) raw += chunk;
  const method = request.url.split("/").at(-1);
  const body = raw && request.headers["content-type"]?.includes("application/json") ? JSON.parse(raw) : {};
  let result = true;
  if (method === "getUpdates") { await sleep(30); result = updates.splice(0); }
  if (method === "sendMessage") {
    result = { message_id: ++message, chat: { id: 1 }, text: body.text };
    if (body.reply_markup?.inline_keyboard?.flat().some(button => button.callback_data === "submit")) {
      const card = { id: message, text: body.text, keyboard: body.reply_markup.inline_keyboard };
      cards.push(card);
      if (autoAnswer) answer(card);
    }
  }
  if (method === "sendDocument" || method === "sendPhoto") {
    result = { message_id: ++message, chat: { id: 1 } };
  }
  response.setHeader("content-type", "application/json");
  response.end(JSON.stringify({ ok: true, result }));
});
await new Promise(resolve => api.listen(0, "127.0.0.1", resolve));
const config = {
  general: { language: "en", menuBarIcon: "off", popupPrewarm: false, daemonLifecycle: "keepalive", away: { capsule: false, welcomeBack: false } },
  channels: { autoActivation: false, popup: { enabled: false }, telegram: { enabled: true, botToken: "123:private", chatId: "1", apiBaseUrl: `http://127.0.0.1:${api.address().port}` } },
};
const saveConfig = () => fs.writeFileSync(path.join(data, "config.json"), JSON.stringify(config));
function child(args, environment = env, stdio = ["ignore", "pipe", "pipe"]) {
  const process = spawn(binary, [...prefix, ...args], { cwd: root, env: environment, stdio });
  children.add(process); process.once("exit", () => children.delete(process));
  return process;
}
function run(args, environment = cursor) {
  return new Promise((resolve, reject) => {
    const process = child(args, environment); let stdout = "", stderr = "";
    const timeout = setTimeout(() => { process.kill("SIGKILL"); reject(new Error(`CLI timeout ${args[0]}`)); }, 45000);
    process.stdout.on("data", chunk => stdout += chunk); process.stderr.on("data", chunk => stderr += chunk);
    process.once("error", error => { clearTimeout(timeout); reject(error); });
    process.once("close", code => { clearTimeout(timeout); resolve({ code, stdout, stderr }); });
  });
}
async function until(check, label) {
  const deadline = Date.now() + 45000;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch {}
    await sleep(100);
  }
  throw new Error(`Timeout waiting for ${label}; evidence ${root}`);
}
function ipc(payload, type) {
  return new Promise((resolve, reject) => {
    const socket = createConnection(path.join(data, "daemon.sock"));
    const timeout = setTimeout(() => { socket.destroy(); reject(new Error("IPC timeout")); }, 1500);
    socket.on("error", error => { clearTimeout(timeout); reject(error); });
    createInterface({ input: socket }).on("error", () => {}).on("line", line => {
      const value = JSON.parse(line);
      if (value.type === type) { clearTimeout(timeout); socket.destroy(); resolve(value); }
    });
    socket.once("connect", () => socket.write(`${JSON.stringify(payload)}\n`));
  });
}
const away = op => ipc({ type: "away", op }, "away");
async function start() {
  const log = fs.openSync(path.join(root, "daemon.log"), "a");
  daemon = child(["daemon", "run"], env, ["ignore", log, log]); fs.closeSync(log);
  await until(() => ipc({ type: "status" }, "status"), "daemon startup");
}
const ok = result => { assert.equal(result.code, 0, result.stderr); return result.stdout; };
function records() {
  const result = spawnSync("sqlite3", [path.join(data, "state/requests.sqlite3"), "SELECT payload FROM metadata WHERE key LIKE 'owned/cursor-away-report/%' ORDER BY key"], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim().split("\n").filter(Boolean).map(JSON.parse);
}
const unread = () => records().filter(record => record.effect === "waiting");
const receipt = output => assert.match(output, /\[cursor_away_report_saved\][\s\S]*\[user_confirmed_end_turn\]/);
async function markRead(card) {
  answer(card);
  await until(() => unread().length === 0, "read acknowledgement");
}

try {
  saveConfig(); await start(); await away({ op: "start" });
  assert.doesNotMatch(ok(await run(["DEFAULT OFF", "-q", "Continue?", "-o", "ack", "--escalate", "done"])), /cursor_away_report_saved/);
  assert.equal(cards.length, 1); assert.equal(records().length, 0);
  console.log("PASS default off retains ordinary escalation");

  config.cursorDesktop = { endEscalationsWhileAway: true }; saveConfig(); await sleep(2500);
  autoAnswer = false;
  const attachment = path.join(root, "report.txt"); fs.writeFileSync(attachment, "private evidence");
  receipt(ok(await run(["SAVED REPORT", "-q", "Deploy?", "-o", "Deploy", "-f", attachment, "--escalate", "complete"])));
  await until(() => cards.length === 2, "unread card");
  assert.equal(unread().length, 1);
  const saved = unread()[0];
  assert.match(JSON.stringify(saved.task), /SAVED REPORT/);
  assert.match(JSON.stringify(saved.task), /Deploy\?/);
  assert.match(JSON.stringify(saved.task), /report.txt/);
  assert.match(cards.at(-1).text, /Unread report/);
  const choices = cards.at(-1).keyboard.flat().filter(button => button.callback_data === "toggle:0");
  assert.equal(choices.length, 1); assert.match(cards.at(-1).text, /Read/);
  console.log("PASS ordinary ask returns before acknowledgement and preserves report content");

  config.cursorDesktop.endEscalationsWhileAway = false; saveConfig(); await sleep(2500);
  await away({ op: "end" });
  const previous = daemon;
  ok(await run(["daemon", "stop", "--force"], env));
  await until(() => previous.exitCode !== null, "explicit stop");
  assert.equal(unread().length, 1); await sleep(500);
  assert.ok(!fs.existsSync(path.join(data, "daemon.sock")), "owned reports must not auto-start a stopped daemon");
  await start();
  await until(() => cards.length === 3, "restored unread card");
  assert.equal(unread().length, 1);
  await markRead(cards.at(-1));
  console.log("PASS unread survives caller exit, disabled preference, away ending and daemon stop/start");

  config.cursorDesktop.endEscalationsWhileAway = true; saveConfig(); await sleep(2500);
  await away({ op: "start" });
  const count = records().length;
  ok(await run(["todo", "add", "--auto", "PRIVATE AUTOMATIC WORK"]));
  const redirect = ok(await run(["QUEUED REPORT", "--escalate", "done"]));
  assert.match(redirect, /Automatic project todos remain/); assert.doesNotMatch(redirect, /cursor_away_report_saved/);
  assert.equal(records().length, count);
  const next = ok(await run(["--whats-next", "--escalate", "done"]));
  assert.match(next, /PRIVATE AUTOMATIC WORK/); assert.doesNotMatch(next, /cursor_away_report_saved/);
  receipt(ok(await run(["--whats-next", "QUEUE COMPLETE", "--escalate", "done"])));
  await until(() => unread().length === 1 && cards.length === 4, "post-todo report");
  await markRead(cards.at(-1));
  console.log("PASS automatic todos precede reports for ask and whats_next");

  await away({ op: "session", sessionKey: "sid:private-session", setting: "mute" });
  assert.match(ok(await run(["MUTED", "--escalate", "done"])), /turned AskHuman off/);
  await away({ op: "session", sessionKey: "sid:private-session", setting: "follow" });
  assert.match(ok(await run(["LEGACY", "--escalate"])), /did not include a reason/);
  autoAnswer = true;
  const codex = { ...cursor, ASKHUMAN_MCP_AGENT_KIND: "codex", ASKHUMAN_MCP_AGENT_SESSION_ID: "private-codex" };
  assert.doesNotMatch(ok(await run(["CODEX", "-o", "ack", "--escalate", "done"], codex)), /cursor_away_report_saved/);
  console.log("PASS mute, legacy escalation and other-agent isolation");

  // Bind a real MCP tools/call through the same one-time private token format as the hook.
  const mcp = child(["mcp"], env, ["pipe", "pipe", "pipe"]);
  mcp.stderr.pipe(fs.createWriteStream(path.join(root, "mcp.log")));
  const pending = new Map(); let rpcId = 0;
  createInterface({ input: mcp.stdout }).on("line", line => {
    const reply = JSON.parse(line), waiter = pending.get(reply.id);
    if (waiter) { pending.delete(reply.id); clearTimeout(waiter.timer); reply.error ? waiter.reject(new Error(JSON.stringify(reply.error))) : waiter.resolve(reply.result); }
  });
  function rpc(method, params) {
    return new Promise((resolve, reject) => {
      const id = ++rpcId, timer = setTimeout(() => { pending.delete(id); reject(new Error(`MCP timeout ${method}`)); }, 45000);
      pending.set(id, { resolve, reject, timer });
      mcp.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
  }
  await rpc("initialize", { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "private-report-smoke", version: "1" } });
  mcp.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n');
  autoAnswer = false;
  for (const name of ["ask", "whats_next"]) {
    const token = randomUUID(), directory = path.join(data, "state/session-tokens");
    fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
    fs.writeFileSync(path.join(directory, `${token}.json`), JSON.stringify({ agent_kind: "cursor", session_id: "private-mcp", tool_name: name, expires_at_ms: Date.now() + 30000 }), { mode: 0o600 });
    const before = cards.length;
    const result = await rpc("tools/call", { name, arguments: { message: `MCP ${name} report`, escalate: "complete", __askhuman_session_token_v1: token } });
    assert.ok(!result.isError, JSON.stringify(result));
    receipt(result.content.filter(item => item.type === "text").map(item => item.text).join("\n"));
    await until(() => cards.length === before + 1, `MCP ${name} card`);
    await markRead(cards.at(-1));
  }
  console.log("PASS trusted MCP ask and whats_next return end receipts before Read");
  await away({ op: "end" });
  autoAnswer = true;
  assert.doesNotMatch(ok(await run(["NORMAL", "-o", "ack", "--escalate", "done"])), /cursor_away_report_saved/);
  assert.equal(unread().length, 0);
  console.log(`PASS normal-mode isolation; private evidence ${root}`);
} finally {
  for (const process of children) process.kill("SIGTERM");
  await sleep(300); for (const process of children) process.kill("SIGKILL");
  api.closeAllConnections(); await new Promise(resolve => api.close(resolve));
}
