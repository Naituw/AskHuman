import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { askArguments, capture, workerEnvironment } from "./development/runtime.mjs";
import { guard, isTurnAbort, monitor, turnMetadata } from "./development/caller.mjs";

const bridge = fileURLToPath(new URL("./dev.mjs", import.meta.url));

function fixture(t) {
  const directory = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "ah-project-test-")));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const home = path.join(directory, "home");
  fs.mkdirSync(home);
  const config = path.join(directory, "gitconfig");
  fs.writeFileSync(config, "");
  const env = { ...workerEnvironment(), HOME: home, USERPROFILE: home, GIT_CONFIG_GLOBAL: config, GIT_CONFIG_NOSYSTEM: "1" };
  function tree(name) {
    const root = path.join(directory, name);
    fs.mkdirSync(root);
    assert.equal(spawnSync("git", ["init", "--quiet", root], { env }).status, 0);
    return root;
  }
  return { home, env, tree };
}

async function connection(t, root, env) {
  const child = spawn(process.execPath, [bridge, "mcp", "--worktree", root], { env, stdio: ["pipe", "pipe", "pipe"] });
  let stderr = "";
  child.stderr.on("data", (data) => { stderr += data; });
  const awaiting = new Map();
  const lines = createInterface({ input: child.stdout });
  lines.on("line", (line) => {
    const message = JSON.parse(line);
    if (message.id != null) { awaiting.get(message.id)?.(message); awaiting.delete(message.id); }
  });
  const closed = new Promise((resolve) => child.once("close", (code) => resolve({ code, stderr })));
  t.after(async () => {
    child.stdin.end();
    const timeout = setTimeout(() => child.kill("SIGKILL"), 3000);
    const result = await closed;
    clearTimeout(timeout);
    assert.equal(result.code, 0, result.stderr);
  });
  let id = 0;
  async function call(method, params) {
    const requestId = ++id;
    const result = new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error(`MCP ${method} timed out: ${stderr}`)), 10000);
      awaiting.set(requestId, (response) => { clearTimeout(timeout); resolve(response); });
    });
    child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id: requestId, method, params })}\n`);
    return result;
  }
  const initialize = await call("initialize", { protocolVersion: "2025-11-25", capabilities: {}, clientInfo: { name: "project-regression", version: "1" } });
  assert.ok(initialize.result, JSON.stringify(initialize));
  child.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n');
  return call;
}

test("development workers never inherit the daily client identity or provider keys", () => {
  const result = workerEnvironment({ HOME: "/home/example", PATH: "/bin", ASKHUMAN_MCP_AGENT_SESSION_ID: "daily", ASKHUMAN_DEV_TEST_PROFILE: "old", CODEX_THREAD_ID: "daily", CLAUDE_CODE_SESSION_ID: "daily", OPENAI_API_KEY: "fixture", XAI_API_KEY: "fixture" });
  assert.deepEqual(result, { HOME: "/home/example", PATH: "/bin", ASKHUMAN_NO_KEYCHAIN: "1" });
});

test("structured verification keeps question and option boundaries without a shell", () => {
  assert.deepEqual(askArguments({ message: "description", questions: [{ question: "first?", options: [{ text: "a; $(not a shell)", recommended: true }, { text: "b" }] }, { question: "second?" }], files: ["/path with spaces/image.png"] }), ["description", "-q", "first?", "-o!", "a; $(not a shell)", "-o", "b", "-q", "second?", "-f", "/path with spaces/image.png"]);
});

test("a project MCP connection lists only developer tools before any Dev build exists", async (t) => {
  const fixtureValue = fixture(t);
  const root = fixtureValue.tree("worktree");
  const call = await connection(t, root, fixtureValue.env);
  const response = await call("tools/list", {});
  assert.deepEqual(response.result.tools.map((tool) => tool.name), ["dev_ask"]);
  const schema = JSON.stringify(response.result.tools[0].inputSchema);
  assert.ok(!schema.includes('"$ref"'));
  assert.ok(!schema.includes("__askhuman_session_token"));
  assert.ok(!fs.existsSync(path.join(root, ".askhuman-dev")));
});

test("project verification rejects another worktree before launching any program", async (t) => {
  const fixtureValue = fixture(t);
  const root = fixtureValue.tree("worktree-a");
  const other = fixtureValue.tree("worktree-b");
  const call = await connection(t, root, fixtureValue.env);
  const response = await call("tools/call", { name: "dev_ask", arguments: { worktree: other, message: "must never display" } });
  assert.equal(response.result.isError, true);
  assert.match(response.result.content[0].text, /another worktree/);
  assert.ok(!fs.existsSync(path.join(root, ".askhuman-dev")));
  assert.ok(!fs.existsSync(path.join(other, ".askhuman-dev")));
});

test("cancellation terminates the verification process instead of returning a human answer", async () => {
  const controller = new AbortController();
  const result = capture(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { signal: controller.signal });
  controller.abort();
  await assert.rejects(result, /cancelled/);
});

test("background origins and delegated calls cannot create a development interaction", (t) => {
  const { home } = fixture(t);
  for (const source of ["system", "ambient_suggestions"]) {
    assert.throws(() => guard({ "x-codex-turn-metadata": JSON.stringify({ thread_source: source }) }, home), /background thread/);
  }
  fs.mkdirSync(path.join(home, "state/codex-delegated"), { recursive: true });
  fs.writeFileSync(path.join(home, "state/codex-delegated/session-a.json"), JSON.stringify({ v: 1, signal: "exec_process" }));
  assert.throws(() => guard({ "x-codex-turn-metadata": { thread_source: "user", session_id: "session-a" } }, home), /non-interactive/);
  guard({ "x-codex-turn-metadata": { thread_source: "user", session_id: "session-b" } }, home);
  assert.deepEqual(turnMetadata({ "x-codex-turn-metadata": '{"turn_id":"one"}' }), { turn_id: "one" });
});

test("native cancellation matches the exact appended turn and handles partial lines", async (t) => {
  const { home } = fixture(t);
  const sessions = path.join(home, "sessions/2026/10/31");
  fs.mkdirSync(sessions, { recursive: true });
  const rollout = path.join(sessions, "rollout-fixture-session-a.jsonl");
  const abort = (id) => JSON.stringify({ type: "event_msg", payload: { type: "turn_aborted", turn_id: id } });
  assert.ok(isTurnAbort(abort("one"), "one"));
  assert.ok(!isTurnAbort(abort("two"), "one"));
  fs.writeFileSync(rollout, `${abort("one")}\n`);
  const controller = new AbortController();
  const stop = monitor({ "x-codex-turn-metadata": { session_id: "session-a", turn_id: "one" } }, controller, home);
  t.after(stop);
  fs.appendFileSync(rollout, `${abort("two")}\n${abort("one").slice(0, -1)}`);
  await new Promise((resolve) => setTimeout(resolve, 150));
  assert.equal(controller.signal.aborted, false);
  fs.appendFileSync(rollout, "}\n");
  await new Promise((resolve) => setTimeout(resolve, 150));
  assert.equal(controller.signal.aborted, true);
});
