import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import fs from "node:fs";
import vm from "node:vm";
import { test } from "node:test";
import ts from "typescript";

const template = fs.readFileSync(new URL("../src-tauri/resources/integrations/pi/index.ts", import.meta.url), "utf8");
function fixture(flags) {
  const calls = [], handlers = new Map();
  const source = template.replace("__ASKHUMAN_CONFIG_JSON__", JSON.stringify({ executable: "/test/AskHuman", programName: "AskHuman", ...flags }));
  const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS }, reportDiagnostics: true });
  assert.equal(compiled.diagnostics.length, 0);
  const fakeSpawn = (executable, args, options) => {
    const child = new EventEmitter();
    child.stdout = new EventEmitter(); child.stdout.setEncoding = () => {};
    child.stderr = { resume() {} }; child.stdin = new EventEmitter();
    child.stdin.end = raw => {
      calls.push({ executable, args, options, payload: JSON.parse(raw) });
      queueMicrotask(() => child.emit("close", 0));
    };
    child.kill = () => { child.killed = true; child.emit("close", 0); };
    return child;
  };
  const exports = {};
  vm.runInNewContext(compiled.outputText, { exports, require(name) { assert.equal(name, "node:child_process"); return { spawn: fakeSpawn }; }, process: { env: {} }, setTimeout, clearTimeout });
  let reasoning = "high", usage = { tokens: 12345, contextWindow: 200000, percent: 6.1725 };
  const ctx = { cwd: "/project/a", model: { id: "native-model", name: "Display name" }, sessionManager: { getSessionId: () => "exact-session", getSessionFile: () => "/sessions/exact.jsonl" }, getContextUsage: () => usage };
  exports.default({ on: (event, handler) => handlers.set(event, handler), getThinkingLevel: () => reasoning });
  return { calls, ctx, emit: (event, payload = {}) => handlers.get(event)?.(payload, ctx), setUsage: value => { usage = value; }, setReasoning: value => { reasoning = value; } };
}

test("Pi reports exact native context even with lifecycle disabled and preserves AskHuman bash timeout", async () => {
  const f = fixture({ cli: true, lifecycle: false, stop: false });
  await f.emit("session_start");
  let report = f.calls.at(-1);
  assert.deepEqual(Array.from(report.args), ["__session-info", "pi"]);
  assert.equal(report.payload.session_id, "exact-session");
  assert.equal(report.payload.cwd, "/project/a");
  assert.equal(report.payload.native_context.model, "native-model");
  assert.equal(report.payload.native_context.usage.tokens, 12345);
  assert.equal(report.options.env.PI_SESSION_ID, "exact-session");
  const ask = { toolName: "bash", input: { command: "cd /tmp && AskHuman -q test", timeout: 5 } };
  await f.emit("tool_call", ask); assert.equal(ask.input.timeout, 86400);
  const other = { toolName: "bash", input: { command: "echo test", timeout: 5 } };
  await f.emit("tool_call", other); assert.equal(other.input.timeout, 5);
  f.setUsage(undefined); await f.emit("session_compact");
  assert.equal(f.calls.at(-1).payload.native_context.usage, null);
  f.ctx.model = { id: "next-model" }; await f.emit("model_select");
  assert.equal(f.calls.at(-1).payload.native_context.model, "next-model");
  f.setReasoning("low"); await f.emit("thinking_level_select");
  assert.equal(f.calls.at(-1).payload.native_context.reasoning, "low");
});

test("Pi native reports follow the existing extension capabilities", async () => {
  const disabled = fixture({ cli: false, lifecycle: false, stop: false });
  await disabled.emit("session_start"); assert.equal(disabled.calls.length, 0);
  const lifecycle = fixture({ cli: false, lifecycle: true, stop: false });
  await lifecycle.emit("session_start");
  assert.deepEqual(lifecycle.calls.map(call => call.args[0]), ["__session-info", "__agent-hook"]);
});
