// Caller metadata stays in this controller; it is never passed to the Dev worker.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";

export function turnMetadata(meta) {
  let value = meta?.["x-codex-turn-metadata"];
  if (typeof value === "string") { try { value = JSON.parse(value); } catch { return null; } }
  return value && typeof value === "object" ? value : null;
}

function boundedJson(file) {
  try { if (fs.statSync(file).size <= 256 * 1024) return JSON.parse(fs.readFileSync(file, "utf8")); } catch { /* Missing caller metadata is inconclusive. */ }
  return null;
}

export function guard(meta, home = path.join(os.homedir(), ".askhuman")) {
  const turn = turnMetadata(meta);
  if (!turn) return;
  if (["system", "ambient_suggestions"].includes(turn.thread_source)) {
    throw new Error("AskHuman is disabled for this system-generated background thread. Do not retry or contact the human; finish the non-interactive output directly.");
  }
  const id = turn.session_id ?? turn.sessionId ?? meta?.sessionId ?? meta?.threadId;
  if (typeof id !== "string" || !id.trim() || boundedJson(path.join(home, "delegated-preferences.json"))?.codex === true) return;
  const stem = /^[a-zA-Z0-9_-]{1,64}$/.test(id) ? id : createHash("sha256").update(id).digest("hex").slice(0, 32);
  const marker = boundedJson(path.join(home, "state", "codex-delegated", `${stem}.json`));
  if (marker?.v === 1 && ["transcript_null", "exec_process", "rollout_source", "rollout_originator"].includes(marker.signal)) {
    throw new Error("AskHuman is not used in this non-interactive Codex run. Do not retry or contact the human; return the result directly.");
  }
}

export function isTurnAbort(line, turnId) {
  try {
    const value = JSON.parse(line);
    return value.type === "event_msg" && value.payload?.type === "turn_aborted" && value.payload.turn_id === turnId;
  } catch { return false; }
}

function rollout(directory, sessionId, depth = 4, budget = { left: 30000 }) {
  let entries;
  try { entries = fs.readdirSync(directory, { withFileTypes: true }); } catch { return null; }
  for (const entry of entries) {
    if (--budget.left < 0) return null;
    const file = path.join(directory, entry.name);
    if (entry.isFile() && entry.name.startsWith("rollout-") && entry.name.endsWith(`-${sessionId}.jsonl`)) return file;
    if (depth > 0 && entry.isDirectory()) { const found = rollout(file, sessionId, depth - 1, budget); if (found) return found; }
  }
  return null;
}

// Capture the offset before launching the worker; only events appended for this exact turn abort it.
export function monitor(meta, controller, codexHome = process.env.CODEX_HOME || path.join(os.homedir(), ".codex")) {
  const turn = turnMetadata(meta);
  const sessionId = turn?.session_id ?? turn?.sessionId;
  const turnId = turn?.turn_id ?? turn?.turnId;
  if (typeof sessionId !== "string" || !/^[a-zA-Z0-9_-]{1,128}$/.test(sessionId) || typeof turnId !== "string" || !turnId) return () => {};
  const file = rollout(path.join(codexHome, "sessions"), sessionId);
  if (!file) return () => {};
  let offset;
  try { offset = fs.statSync(file).size; } catch { return () => {}; }
  let pending = "";
  const timer = setInterval(() => {
    if (controller.signal.aborted) return;
    let fd;
    try {
      fd = fs.openSync(file, "r");
      const size = fs.fstatSync(fd).size;
      if (size < offset) { offset = 0; pending = ""; }
      const bytes = Buffer.alloc(Math.min(1024 * 1024, size - offset));
      const read = fs.readSync(fd, bytes, 0, bytes.length, offset);
      offset += read;
      pending += bytes.subarray(0, read).toString("utf8");
      let end;
      while ((end = pending.indexOf("\n")) >= 0) {
        const line = pending.slice(0, end); pending = pending.slice(end + 1);
        if (isTurnAbort(line, turnId)) { controller.abort(); clearInterval(timer); return; }
      }
      // A giant non-event line cannot exhaust the controller's memory.
      if (pending.length > 2 * 1024 * 1024) pending = "";
    } catch { /* Rotation or transient filesystem failure: try the next bounded poll. */ }
    finally { if (fd !== undefined) fs.closeSync(fd); }
  }, 100);
  timer.unref();
  return () => clearInterval(timer);
}
