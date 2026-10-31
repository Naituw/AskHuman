import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import fs from "node:fs";
import { randomUUID } from "node:crypto";
import vm from "node:vm";
import { test } from "node:test";
import ts from "typescript";

const template = fs.readFileSync(new URL("../src-tauri/resources/integrations/pi/index.ts", import.meta.url), "utf8");
function fixture(flags, control = {}) {
  const calls = [], handlers = new Map(), sent = [], timers = new Map();
  let idle = true, nextOperation = control.operation, stopReply = control.stopReply;
  const timerSet = control.manual ? (fn, ms) => { const id = {}; timers.set(id, { fn, ms }); return id; } : setTimeout;
  const timerClear = control.manual ? id => timers.delete(id) : clearTimeout;
  const source = template.replace("__ASKHUMAN_CONFIG_JSON__", JSON.stringify({ executable: "/test/AskHuman", programName: "AskHuman", ...flags }));
  const compiled = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS }, reportDiagnostics: true });
  assert.equal(compiled.diagnostics.length, 0);
  const fakeSpawn = (executable, args, options) => {
    const child = new EventEmitter();
    child.stdout = new EventEmitter(); child.stdout.setEncoding = () => {};
    child.stderr = { resume() {} }; child.stdin = new EventEmitter();
    child.stdin.end = raw => {
      calls.push({ executable, args, options, payload: JSON.parse(raw) });
      queueMicrotask(() => {
        if (args[0] === "__agent-control" && !JSON.parse(raw).operationId) {
          if (control.busyBeforeDelivery) idle = false;
          child.stdout.emit("data", JSON.stringify({ operation: nextOperation }));
          nextOperation = undefined;
        }
        if (args[0] === "__stop-hook") { child.stdout.emit("data", JSON.stringify(stopReply ?? {})); stopReply = undefined; }
        child.emit("close", 0);
      });
    };
    child.kill = () => { child.killed = true; child.emit("close", 0); };
    return child;
  };
  const exports = {};
  vm.runInNewContext(compiled.outputText, { exports, require(name) { if (name === "node:crypto") return { randomUUID }; assert.equal(name, "node:child_process"); return { spawn: fakeSpawn }; }, process: { env: {}, pid: 123 }, setTimeout: timerSet, clearTimeout: timerClear });
  let reasoning = "high", usage = { tokens: 12345, contextWindow: 200000, percent: 6.1725 };
  const ctx = { cwd: "/project/a", model: { id: "native-model", name: "Display name" }, sessionManager: { getSessionId: () => "exact-session", getSessionFile: () => "/sessions/exact.jsonl" }, getContextUsage: () => usage, isIdle: () => idle, hasPendingMessages: () => false };
  exports.default({ on: (event, handler) => handlers.set(event, handler), getThinkingLevel: () => reasoning,
    async sendUserMessage(text) { sent.push(text); await handlers.get("agent_start")?.({}, ctx); }
  });
  return { calls, ctx, sent, async poll() {
    const entry = [...timers].find(([, value]) => value.ms === 250 || value.ms === 1500);
    assert.ok(entry, "idle control poll is scheduled"); timers.delete(entry[0]); entry[1].fn();
    await new Promise(setImmediate);
  }, emit: (event, payload = {}) => handlers.get(event)?.(payload, ctx), setUsage: value => { usage = value; }, setReasoning: value => { reasoning = value; } };
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
  await lifecycle.emit("session_shutdown");
  assert.deepEqual(lifecycle.calls.map(call => call.args[0]), ["__session-info", "__agent-hook", "__agent-hook"]);
});


test("Pi idle control dispatches once and associates the actual new round with its operation", async () => {
  const f = fixture({ lifecycle: true, stop: false }, { manual: true, operation: { id: "phone-operation", text: "Continue from progress", files: [] } });
  await f.emit("session_start"); await f.poll();
  assert.equal(f.sent.length, 1);
  const start = f.calls.find(call => call.args[2] === "turn-start");
  assert.equal(start.payload.control_operation_id, "phone-operation");
  assert.match(start.payload.turn_id, /:[0-9]+$/);
  await f.poll(); assert.equal(f.sent.length, 1);
  await f.emit("agent_end", { messages: [{ role: "assistant", stopReason: "aborted", content: [] }] });
  await f.emit("agent_settled");
  const end = f.calls.findLast(call => call.args[2] === "turn-end");
  assert.equal(end.payload.stop_reason, "aborted"); assert.equal(end.payload.turn_id, start.payload.turn_id);
  await f.emit("session_shutdown");
});

test("Pi rechecks native idle state before accepting a dispatched command", async () => {
  const f = fixture({ lifecycle: true, stop: false }, { manual: true, busyBeforeDelivery: true, operation: { id: "phone-operation", text: "Continue", files: [] } });
  await f.emit("session_start"); await f.poll(); assert.equal(f.sent.length, 0);
  const ack = f.calls.find(call => call.payload.operationId === "phone-operation");
  assert.equal(ack.payload.status, "rejected"); await f.emit("session_shutdown");
});


test("Pi managed Stop continuation preserves the phone round until its actual result", async () => {
  const f = fixture({ lifecycle: true, stop: true }, { manual: true, operation: { id: "phone-operation", text: "Continue", files: [] }, stopReply: { followup_message: "Continue the existing task" } });
  await f.emit("session_start"); await f.poll();
  const original = f.calls.find(call => call.args[2] === "turn-start").payload.turn_id;
  const finished = { messages: [{ role: "assistant", stopReason: "stop", content: [] }] };
  await f.emit("agent_end", finished); await f.emit("agent_settled"); await new Promise(setImmediate);
  const starts = f.calls.filter(call => call.args[2] === "turn-start");
  assert.equal(starts.length, 2); assert.equal(starts[1].payload.turn_id, original);
  assert.equal(f.calls.filter(call => call.args[2] === "turn-end").length, 0);
  await f.emit("agent_end", finished); await f.emit("agent_settled"); await new Promise(setImmediate);
  const result = f.calls.findLast(call => call.args[2] === "turn-end");
  assert.equal(result.payload.turn_id, original); assert.equal(result.payload.stop_reason, "stop");
  await f.emit("session_shutdown");
});
