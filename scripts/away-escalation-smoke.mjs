// Real CLI, MCP server and daemon with private HOME and a loopback Telegram transport.
// No model, real Agent, GUI or remote bot is used.
// Usage: node scripts/away-escalation-smoke.mjs [path/to/AskHuman]
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { createConnection } from "node:net";
import { createInterface } from "node:readline";

if (process.platform === "win32") { console.log("SKIP: private HOME smoke uses Unix sockets"); process.exit(0); }
const original = path.resolve(process.argv[2] ?? "src-tauri/target/local-install/AskHuman");
const temp = fs.realpathSync(fs.mkdtempSync(path.join(process.platform === "darwin" ? "/tmp" : os.tmpdir(), "aha-")));
const home = path.join(temp, "user"), ah = path.join(temp, "ah"), binary = path.join(temp, "AskHuman");
for (const directory of [home, ah]) fs.mkdirSync(directory, { recursive: true });
fs.copyFileSync(original, binary); fs.chmodSync(binary, 0o700);
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(ASKHUMAN_|CODEX_|CLAUDE_|CURSOR_|GROK_|PI_SESSION|PI_CODING)/.test(key)) delete env[key];
Object.assign(env, { HOME: home, ASKHUMAN_HOME: ah, ASKHUMAN_NO_KEYCHAIN: "1", GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: path.join(temp, "gitconfig") });
fs.writeFileSync(env.GIT_CONFIG_GLOBAL, "");
const cliEnv = { ...env, ASKHUMAN_FROM_MCP: "1", ASKHUMAN_MCP_INSTANCE_ID: "smoke-cli", ASKHUMAN_MCP_AGENT_KIND: "codex", ASKHUMAN_MCP_AGENT_SESSION_ID: "smoke-cli" };
const children = new Set(), cards = [], messages = [], updates = [];
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
let updateId = 0, messageId = 0, autoAnswer = true;
const callback = (id, data) => ({ update_id: ++updateId, callback_query: { id: String(updateId), data, from: { id: 1 }, message: { message_id: id, chat: { id: 1 } } } });
const answer = card => updates.push(callback(card.id, "toggle:0"), callback(card.id, "submit"));
const api = http.createServer(async (request, response) => {
  let raw = ""; for await (const part of request) raw += part;
  const body = raw ? JSON.parse(raw) : {}, method = request.url.split("/").at(-1);
  let result = true;
  if (method === "getUpdates") { await sleep(30); result = updates.splice(0); }
  else if (method === "sendMessage") {
    const id = ++messageId; result = { message_id: id, chat: { id: 1 }, text: body.text };
    messages.push(body.text ?? "");
    if (body.reply_markup?.inline_keyboard?.flat().some(button => button.callback_data === "submit")) {
      const card = { id, text: body.text }; cards.push(card);
      if (autoAnswer) answer(card);
    }
  }
  response.setHeader("content-type", "application/json"); response.end(JSON.stringify({ ok: true, result }));
});
await new Promise(resolve => api.listen(0, "127.0.0.1", resolve));
function child(args, childEnv = env, stdio = ["pipe", "pipe", "pipe"]) {
  const process = spawn(binary, args, { cwd: temp, env: childEnv, stdio });
  children.add(process); process.once("exit", () => children.delete(process)); return process;
}
function run(args, childEnv = cliEnv) {
  return new Promise((resolve, reject) => {
    const process = child(args, childEnv); let stdout = "", stderr = "";
    const timer = setTimeout(() => { process.kill("SIGKILL"); reject(new Error(`CLI timeout: ${args[0]}`)); }, 45000);
    process.stdout.on("data", chunk => { stdout += chunk; }); process.stderr.on("data", chunk => { stderr += chunk; });
    process.once("error", error => { clearTimeout(timer); reject(error); });
    process.once("close", code => { clearTimeout(timer); resolve({ code, stdout, stderr }); });
    process.stdin.end();
  });
}
async function until(fn, what) {
  const end = Date.now() + 30000;
  while (Date.now() < end) { if (await fn()) return; await sleep(50); }
  throw new Error(`Timeout: ${what}`);
}
function ipc(payload, type) {
  return new Promise((resolve, reject) => {
    const socket = createConnection(path.join(ah, "daemon.sock"));
    const timer = setTimeout(() => { socket.destroy(); reject(new Error(`IPC timeout: ${type}`)); }, 3000);
    socket.once("error", error => { clearTimeout(timer); reject(error); });
    createInterface({ input: socket }).on("error", () => {}).on("line", line => {
      const value = JSON.parse(line);
      if (value.type === type) { clearTimeout(timer); socket.destroy(); resolve(value); }
    });
    socket.once("connect", () => socket.write(`${JSON.stringify(payload)}\n`));
  });
}
const away = op => ipc({ type: "away", op }, "away");
const history = () => fs.readFileSync(path.join(ah, "history.jsonl"), "utf8").trim().split("\n").filter(Boolean).map(JSON.parse);
const ok = result => { assert.equal(result.code, 0, result.stderr); return result.stdout; };
const textOf = result => result.content?.filter(item => item.type === "text").map(item => item.text).join("\n") ?? "";

try {
  fs.writeFileSync(path.join(ah, "config.json"), JSON.stringify({
    general: { language: "en", popupPrewarm: false, menuBarIcon: "off", away: { capsule: false, welcomeBack: false } },
    channels: { autoActivation: false, popup: { enabled: false }, telegram: { enabled: true, botToken: "123:fixture", chatId: "1", apiBaseUrl: `http://127.0.0.1:${api.address().port}` } },
  }));
  const log = fs.openSync(path.join(temp, "daemon.log"), "a");
  child(["daemon", "run"], env, ["ignore", log, log]); fs.closeSync(log);
  await until(async () => { try { return await ipc({ type: "status" }, "status"); } catch { return false; } }, "daemon start");
  await away({ op: "start", note: "SMOKE away note" });

  // Legacy true, empty reasons and ordinary questions share one anti-lockout counter.
  for (const [index, args] of [[0, ["--escalate"]], [1, ["--escalate=  "]], [2, []]]) {
    const output = ok(await run(["Q", "-o", "ack", ...args]));
    assert.match(output, /original objective/); assert.match(output, /SMOKE away note/);
    if (index < 2) assert.match(output, /did not include a reason/);
    assert.equal(cards.length, 0, "automatic replies must not reach the transport");
  }
  ok(await run(["Fourth", "-o", "ack", "--escalate"]));
  assert.equal(cards.length, 1); assert.ok(messages.some(text => text.includes("System fallback")));
  assert.match(history().at(-1).message.text, /the agent has not provided a reason/);
  console.log("PASS CLI legacy/empty guidance and fourth-request fallback");

  const reason = '-why "now"\n需要用户决定 *literal* <tag>';
  const body = "CLI REPORT";
  ok(await run([`--escalate=${reason}`, body, "-q", "Continue?", "-o", "ack"]));
  assert.equal(cards.length, 2); assert.ok(messages.some(text => text.includes("Agent-provided escalation reason")));
  assert.ok(messages.some(text => text.includes("需要用户决定")), "the reason itself reaches the transport");
  const stored = history().at(-1).message.text;
  assert.ok(stored.includes('\\*literal\\* \\<tag\\>')); assert.ok(stored.endsWith(body));
  assert.match(stored, /需要用户决定/);
  const beforeReplay = history().length;
  assert.match(ok(await run([`--escalate=${reason}`, body, "-q", "Continue?", "-o", "ack"])), /They were not asked again/);
  assert.equal(cards.length, 2); assert.equal(history().length, beforeReplay);
  console.log("PASS literal CLI reason, shared message history and answer replay");

  // Two live callers get one question and one recorded answer.
  autoAnswer = false;
  const pendingArgs = ["PARALLEL REPORT", "-q", "Wait?", "-o", "ack", "--escalate", "Independent work is complete"];
  const first = run(pendingArgs);
  await until(() => cards.length === 3, "coalesced card");
  const second = run(pendingArgs);
  await until(() => fs.readFileSync(path.join(temp, "daemon.log"), "utf8").includes("coalesced"), "second waiter");
  assert.equal(cards.length, 3); answer(cards.at(-1));
  ok(await first); ok(await second); autoAnswer = true;
  assert.equal(history().filter(entry => entry.message.text.endsWith("PARALLEL REPORT")).length, 1);
  console.log("PASS concurrent retry coalescing");

  await away({ op: "session", sessionKey: "sid:smoke-cli", setting: "mute" });
  assert.match(ok(await run(["Muted", "--escalate", "Need user", "-o", "ack"])), /turned AskHuman off for this session/i);
  assert.equal(cards.length, 3);
  await away({ op: "session", sessionKey: "sid:smoke-cli", setting: "follow" });
  const tooLong = await run(["Q", `--escalate=${"问".repeat(2001)}`]);
  assert.equal(tooLong.code, 1); assert.match(tooLong.stderr, /2000/); assert.equal(cards.length, 3);
  console.log("PASS mute priority and reason length validation");

  // A fresh MCP process advertises strings while accepting old cached clients' bools.
  const mcp = child(["mcp"]); let rpcId = 0;
  const pending = new Map();
  mcp.stderr.pipe(fs.createWriteStream(path.join(temp, "mcp.log")));
  createInterface({ input: mcp.stdout }).on("line", line => {
    const reply = JSON.parse(line), waiter = pending.get(reply.id);
    if (waiter) { pending.delete(reply.id); clearTimeout(waiter.timer); if (reply.error) waiter.reject(new Error(JSON.stringify(reply.error))); else waiter.resolve(reply.result); }
  });
  const rpc = (method, params) => new Promise((resolve, reject) => {
    const id = ++rpcId, timer = setTimeout(() => { pending.delete(id); reject(new Error(`MCP timeout: ${method}`)); }, 45000);
    pending.set(id, { resolve, reject, timer }); mcp.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  });
  await rpc("initialize", { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "away-smoke", version: "1" } });
  mcp.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n');
  const list = await rpc("tools/list", {});
  for (const name of ["ask", "whats_next"]) {
    const schema = list.tools.find(tool => tool.name === name).inputSchema.properties.escalate;
    assert.ok(JSON.stringify(schema).includes('"string"')); assert.ok(!JSON.stringify(schema).includes('"boolean"'));
    assert.equal(schema.description, "Why this request must reach the user now. Use only as instructed by an AskHuman away reply.");
  }
  const call = (name, args) => rpc("tools/call", { name, arguments: args });
  for (const escalate of [true, ""]) {
    const result = await call("ask", { message: "MCP Q", escalate });
    assert.ok(!result.isError); assert.match(textOf(result), /did not include a reason/);
  }
  const next = await call("whats_next", { message: "MCP DONE" });
  assert.match(textOf(next), /do not package it as a separate next task/); assert.equal(cards.length, 3);
  await call("ask", { message: "MCP FALLBACK", questions: [{ question: "Q", options: [{ text: "ack" }] }], escalate: true });
  assert.equal(cards.length, 4); assert.match(history().at(-1).message.text, /System fallback/);
  const done = await call("whats_next", { message: "MCP DONE", escalate: reason });
  assert.ok(!done.isError); assert.equal(cards.length, 5);
  assert.ok(history().at(-1).message.text.endsWith("MCP DONE"));
  assert.match(history().at(-1).message.text, /Agent-provided escalation reason/);
  console.log("PASS fresh MCP schema, legacy inputs, shared guidance, fallback and whats_next reason");

  await away({ op: "end" });
  ok(await run(["NORMAL REPORT", "-q", "Q", "-o", "ack", "--escalate", "Away ended while sending"]));
  assert.match(history().at(-1).message.text, /Away ended while sending/);
  assert.equal(cards.length, 6);
  console.log("PASS reason preservation after away mode ends");
  console.log(`PASS away escalation smoke; private evidence: ${temp}`);
} finally {
  for (const process of children) process.kill("SIGTERM");
  await sleep(300); for (const process of children) process.kill("SIGKILL");
  api.closeAllConnections(); await new Promise(resolve => api.close(resolve));
}
