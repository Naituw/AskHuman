// Actual daemon, Cursor lifecycle hooks, a forged Cursor structured log and a loopback Telegram
// transport. No Cursor, GUI, model or bot: HOME and the log root are temporary directories, so
// nothing here can reach the real Cursor or start one of its requests.
//
//   node scripts/agent-failure-cursor-smoke.mjs [path/to/AskHuman]
//
// Covers docs/specs/agent-failure-inbox.md: the daemon notices a turn Cursor gave up on from its
// log alone, shows one card, keeps that card across a daemon replacement without being held up by
// it, and withdraws it when the Agent moves on.
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { createConnection } from "node:net";
import { createInterface } from "node:readline";
import { randomUUID } from "node:crypto";

if (process.platform === "win32") { console.log("SKIP: private HOME smoke uses Unix sockets"); process.exit(0); }
const original = path.resolve(process.argv[2] ?? "src-tauri/target/local-install/AskHuman");
const temp = fs.realpathSync(fs.mkdtempSync(path.join(process.platform === "darwin" ? "/tmp" : os.tmpdir(), "ahc-")));
const home = path.join(temp, "user"), ah = path.join(temp, "ah"), binary = path.join(temp, "AskHuman");
const logs = path.join(temp, "cursor-logs");
const logDirectory = path.join(logs, "20261103T000000", "window1", "exthost", "anysphere.cursor-always-local");
const logFile = path.join(logDirectory, "Cursor Structured Logs.log");
const cardsFile = path.join(ah, "state", "failure-cards.json");
for (const directory of [home, ah, logDirectory]) fs.mkdirSync(directory, { recursive: true });
fs.copyFileSync(original, binary); fs.chmodSync(binary, 0o700);
fs.writeFileSync(logFile, '2026-11-03 05:00:00.000 [info] {"message":"unrelated"}\n');
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(ASKHUMAN_|CODEX_|CLAUDE_|CURSOR_|GROK_|PI_SESSION|PI_CODING)/.test(key)) delete env[key];
Object.assign(env, {
  HOME: home, ASKHUMAN_HOME: ah, ASKHUMAN_NO_KEYCHAIN: "1", ASKHUMAN_CURSOR_LOG_ROOT: logs,
  GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: path.join(temp, "gitconfig"),
});
fs.writeFileSync(env.GIT_CONFIG_GLOBAL, "");

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
let daemon, update = 0, message = 0;
const updates = [], cards = [], messages = [];
const callback = (id, data) => ({ update_id: ++update, callback_query: { id: String(update), data, from: { id: 1 }, message: { message_id: id, chat: { id: 1 } } } });
const api = http.createServer(async (request, response) => {
  let raw = ""; for await (const part of request) raw += part;
  const body = raw ? JSON.parse(raw) : {}, method = request.url.split("/").at(-1);
  let result = true;
  if (method === "getUpdates") { await sleep(30); result = updates.splice(0); }
  else if (method === "sendMessage") {
    const id = ++message; result = { message_id: id, chat: { id: 1 }, text: body.text };
    messages.push(body.text ?? "");
    if (body.reply_markup?.inline_keyboard?.flat().some(button => button.callback_data === "submit")) cards.push({ id });
  }
  response.setHeader("content-type", "application/json"); response.end(JSON.stringify({ ok: true, result }));
});
await new Promise(resolve => api.listen(0, "127.0.0.1", resolve));

// Never block this process while the daemon runs: it keeps calling the loopback transport above.
function run(args, input, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(binary, args, { cwd: temp, env: options.env ?? env, stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "", stderr = "";
    const timer = setTimeout(() => { child.kill("SIGKILL"); reject(new Error(`${args.join(" ")}: timed out`)); }, 30000);
    child.stdout.on("data", chunk => { stdout += chunk; });
    child.stderr.on("data", chunk => { stderr += chunk; });
    child.on("error", error => { clearTimeout(timer); reject(error); });
    child.on("close", code => {
      clearTimeout(timer);
      if (code === 0) resolve(stdout); else reject(new Error(`${args.join(" ")} exited with ${code}: ${stderr}`));
    });
    child.stdin.end(input ?? "");
  });
}
async function until(fn, what = "fixture") {
  const end = Date.now() + 25000;
  while (Date.now() < end) { try { if (await fn()) return; } catch {} await sleep(50); }
  throw new Error(`${what} timed out`);
}
async function status() {
  return new Promise((resolve, reject) => {
    const socket = createConnection(path.join(ah, "daemon.sock"));
    const timer = setTimeout(() => { socket.destroy(); reject(new Error("status timeout")); }, 3000);
    socket.on("error", error => { clearTimeout(timer); reject(error); });
    createInterface({ input: socket }).on("error", () => {}).on("line", line => {
      const value = JSON.parse(line);
      if (value.type === "status") { clearTimeout(timer); socket.destroy(); resolve(value); }
    });
    socket.on("connect", () => socket.write('{"type":"status"}\n'));
  });
}
const record = async session => JSON.parse(await run(["agents", "monitor", "--json"])).find(entry => entry.sessionId === session);
const hook = (event, payload) => run(["__agent-hook", "cursor", event], JSON.stringify({ cwd: temp, workspace_roots: [temp], ...payload }));
const startSession = session => hook("session-start", { conversation_id: session, hook_event_name: "sessionStart" });
const startTurn = (session, generation) => hook("turn-start", { conversation_id: session, generation_id: generation, hook_event_name: "beforeSubmitPrompt", prompt: "work" });
// Shaped like the line Cursor writes when it gives up on a turn: a time prefix and string values.
const outcome = (session, request, text, retries) => `2026-11-03 05:47:50.114 [info] ${JSON.stringify({
  level: "info", key: "abcd1234", message: "agent.turn.outcome",
  metadata: {
    turn_type: "new", request_id: request, conversation_id: session, outcome: "error", error_type: "transient",
    error_code: "unknown", error_text: text, turn_number: "2", retries_attempted: retries, duration_ms: "1234.5",
  },
})}\n`;
const fail = (session, request, text, retries = "3") => fs.appendFileSync(logFile, outcome(session, request, text, retries));
const saved = () => JSON.parse(fs.readFileSync(cardsFile, "utf8")).cards;
const answer = card => updates.push(callback(card.id, "toggle:0"), callback(card.id, "submit"));
function spawnDaemon() {
  const log = fs.openSync(path.join(temp, "daemon.log"), "a");
  const child = spawn(binary, ["daemon", "run"], { cwd: temp, env, stdio: ["ignore", log, log] });
  fs.closeSync(log); return child;
}
const exited = child => new Promise(resolve => { if (child.exitCode !== null) resolve(child.exitCode); else child.once("exit", resolve); });
const within = (promise, ms, what) => new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error(`${what} timed out`)), ms);
  promise.then(value => { clearTimeout(timer); resolve(value); }, error => { clearTimeout(timer); reject(error); });
});
// The log follower only reads what is written after its first poll, five seconds into a daemon's life.
const tailerReady = () => sleep(7500);
const REASON = "Agent turn stopped after repeated resume attempts made no progress";
const said = pattern => messages.filter(text => pattern.test(text)).length;
const working = session => until(async () => (await record(session))?.state === "working", "a working session");

// `autoActivation` is the "send on demand" switch. The daemon reads it again on every submission,
// so a scenario can flip it without a restart.
const config = autoActivation => JSON.stringify({
  general: { popupPrewarm: false, menuBarIcon: "off" },
  channels: { autoActivation, popup: { enabled: false }, telegram: { enabled: true, botToken: "123:fixture", chatId: "1", apiBaseUrl: `http://127.0.0.1:${api.address().port}` } },
});

try {
  fs.writeFileSync(path.join(ah, "config.json"), config(false));
  await run(["agents", "mode", "cursor", "mcp"]);
  await run(["agents", "stop", "cursor", "on"]);
  daemon = spawnDaemon();
  await until(status, "daemon start");
  await tailerReady();

  // 1. A turn Cursor gave up on, seen only through its log, becomes one card and a failed result.
  const session = randomUUID(), turn = randomUUID();
  await startSession(session); await startTurn(session, turn);
  await working(session);
  fail(session, turn, REASON);
  // A conversation no hook ever registered must not become a session, and a repeated line no card.
  const stranger = randomUUID();
  fail(stranger, randomUUID(), "A conversation this daemon never heard of", "0");
  fail(session, turn, REASON);
  await until(() => cards.length === 1, "first card");
  const reasons = said(new RegExp(REASON));
  assert.ok(reasons >= 1, "the card carries Cursor's own words");
  assert.ok(said(/retried 3 time|已重试 3 次/) >= 1, "and how often Cursor retried");
  const failed = await record(session);
  assert.equal(failed.state, "idle");
  assert.equal(failed.lastTurnResult.outcome, "failed");
  assert.equal(failed.lastTurnResult.source, "cursorLog");
  assert.equal(failed.lastTurnResult.turnId, turn);
  await sleep(6500);
  assert.equal(cards.length, 1, "a repeated line or an unknown conversation cannot raise a card");
  assert.equal(await record(stranger), undefined, "the log alone never creates a session");

  // 2. The unanswered card is on record, and it does not make the daemon wait to be replaced.
  const [entry] = saved();
  assert.equal(saved().length, 1);
  assert.deepEqual([entry.kind, entry.sessionId], ["cursor", session]);
  assert.equal((await status()).activeRequests, 0, "a card the daemon holds is not a request to wait for");

  // 3. A replacement keeps the card: the old daemon leaves at once and the new one asks again.
  daemon.kill("SIGTERM");
  await within(exited(daemon), 15000, "the daemon leaving while it holds a card");
  assert.match(fs.readFileSync(path.join(temp, "daemon.log"), "utf8"), /SIGTERM received; shutting down/);
  assert.deepEqual(saved().map(card => card.sessionId), [session], "the record survives the shutdown");
  daemon = spawnDaemon();
  await until(status, "daemon restart");
  await until(() => cards.length === 2, "restored card");
  assert.ok(said(new RegExp(REASON)) > reasons, "the restored card says the same thing again");
  assert.deepEqual(saved().map(card => [card.sessionId, card.createdMs]), [[session, entry.createdMs]], "a restored card keeps its age");
  assert.equal((await record(session)).lastTurnResult.outcome, "failed");

  // 4. Answering the card settles it for good.
  answer(cards[1]);
  await until(() => !fs.existsSync(cardsFile), "answered card removal");
  assert.equal((await record(session)).lastTurnResult.outcome, "failed", "acknowledging keeps the failure in the Agents list");
  assert.equal(cards.length, 2);

  // 5. The Agent moving on by itself withdraws a card nobody answered.
  const second = randomUUID();
  await startTurn(session, second);
  await working(session);
  await tailerReady();
  fail(session, second, "A second failure");
  await until(() => cards.length === 3, "second failure card");
  assert.equal(saved().length, 1);
  await startTurn(session, randomUUID());
  await until(() => !fs.existsSync(cardsFile), "withdrawn card removal");

  // 6. With the confirmation switch off the log is not followed, and what it said meanwhile is not
  //    replayed when the switch comes back.
  await run(["agents", "stop", "cursor", "off"]);
  const third = randomUUID();
  await startTurn(session, third);
  await sleep(6000);
  fail(session, third, "Failed while the switch was off");
  await sleep(6500);
  assert.equal(cards.length, 3, "nothing is raised while the switch is off");
  await run(["agents", "stop", "cursor", "on"]);
  await sleep(7500);
  assert.equal(cards.length, 3, "history written while off is not replayed");
  assert.equal(fs.existsSync(cardsFile), false);

  // 7. The shape of 2026-11-03: Cursor gives up while an AskHuman question is still waiting. The
  //    question is left alone, and the card follows as soon as it has been answered.
  const asker = randomUUID(), askerTurn = randomUUID();
  await startSession(asker); await startTurn(asker, askerTurn);
  await working(asker);
  const asked = cards.length;
  const question = spawn(binary, ["-q", "Pick one", "-o", "A", "-o", "B"], {
    cwd: temp, env: { ...env, CURSOR_AGENT: "1", CURSOR_CONVERSATION_ID: asker }, stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  question.stdout.on("data", chunk => { output += chunk; });
  const questionExit = exited(question);
  await until(() => cards.length === asked + 1, "the question's own card");
  fail(asker, askerTurn, "Connection failed repeatedly", "0");
  await until(async () => (await record(asker))?.lastTurnResult?.outcome === "failed", "the failure of the asking session");
  await sleep(1500);
  assert.equal(question.exitCode, null, "the question is still waiting for its answer");
  assert.equal(said(/Connection failed repeatedly/), 0, "the failure card waits behind the question");
  answer(cards[asked]);
  assert.equal(await within(questionExit, 15000, "the question's answer"), 0);
  assert.match(output, /\bA\b/, output);
  await until(() => said(/Connection failed repeatedly/) >= 1, "the failure card after the answer");
  // The card reaches the transport as its text and then, a moment later, the message that carries
  // the answer buttons; only that one is counted here.
  await until(() => cards.length === asked + 2, "the failure card's answer buttons");
  answer(cards[asked + 1]);
  await until(() => !fs.existsSync(cardsFile), "settled failure card");

  // 8. The same card with "send on demand" on, which is how the daily setup runs. The card is a
  //    request of the failed session; counting it as that Agent working again would flip the
  //    session out of idle, and the card checks for idle every two seconds and would withdraw
  //    itself. Scenarios 1 to 7 ran with the switch off and could not see that.
  fs.writeFileSync(path.join(ah, "config.json"), config(true));
  const demand = randomUUID(), demandTurn = randomUUID();
  await startSession(demand); await startTurn(demand, demandTurn);
  await working(demand);
  const beforeDemand = cards.length;
  fail(demand, demandTurn, REASON);
  await until(() => cards.length === beforeDemand + 1, "the card with send on demand on");
  await sleep(6000); // several of the card's own checks
  const standing = await record(demand);
  assert.equal(standing.state, "idle", "the card's own request is not the Agent working again");
  assert.equal(standing.lastTurnResult.outcome, "failed");
  assert.deepEqual(saved().map(card => card.sessionId), [demand], "the card is still waiting to be answered");
  answer(cards[beforeDemand]);
  await until(() => !fs.existsSync(cardsFile), "settled card with send on demand on");
  const settled = await record(demand);
  assert.equal(settled.state, "idle");
  assert.equal(settled.lastTurnResult.outcome, "failed", "acknowledging keeps the failure in the Agents list");

  console.log("PASS: forged Cursor log → one card and a failed result; unknown conversations and repeated lines ignored; held card does not keep the daemon; card survives a replacement with its age; answered and withdrawn cards leave the record; switch off stops the follower without replay; a card waits behind a pending question and follows its answer; with send on demand on the card keeps standing until answered");
} catch (error) {
  if (fs.existsSync(path.join(temp, "daemon.log"))) console.error(fs.readFileSync(path.join(temp, "daemon.log"), "utf8").slice(-7000));
  console.error(`cards: ${cards.length}; messages sent to the transport:\n${messages.map((text, index) => `${index + 1}. ${text.replace(/\s+/g, " ").slice(0, 140)}`).join("\n")}`);
  throw error;
} finally {
  if (daemon) {
    daemon.kill();
    await new Promise(resolve => { if (daemon.exitCode !== null) return resolve(); daemon.once("exit", resolve); setTimeout(() => { daemon.kill("SIGKILL"); resolve(); }, 2000).unref(); });
  }
  await new Promise(resolve => api.close(resolve));
  fs.rmSync(temp, { recursive: true, force: true });
}
