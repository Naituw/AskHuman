import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";

export const manifestName = "workspace.json";
const maxOutput = 16 * 1024 * 1024;

// Rust canonical paths can carry a Windows verbatim prefix; Node and Git may omit it.
export function pathKey(value) {
  if (process.platform !== "win32") return value;
  if (value.startsWith("\\\\?\\UNC\\")) value = "\\\\" + value.slice(8);
  else if (value.startsWith("\\\\?\\")) value = value.slice(4);
  return value.toLowerCase();
}
export function physicalPath(value) { return pathKey(fs.realpathSync.native(value)); }

export function worktreeRoot(value) {
  if (!path.isAbsolute(value)) throw new Error("worktree must be an absolute path");
  const root = fs.realpathSync.native(value);
  const git = spawnSync("git", ["-C", root, "rev-parse", "--show-toplevel"], { encoding: "utf8", timeout: 3000 });
  if (git.status !== 0) throw new Error(`cannot identify the project git root: ${(git.stderr || git.error?.message || "git exited without a root").trim().slice(0, 1024)}`);
  if (physicalPath(git.stdout.trim()) !== physicalPath(root)) throw new Error("worktree must identify its git root");
  return root;
}

export function readWorkspace(root) {
  const directory = path.join(root, ".askhuman-dev");
  const file = path.join(directory, manifestName);
  for (const item of [directory, file]) {
    if (fs.lstatSync(item).isSymbolicLink()) throw new Error("linked development manifest requires manual review");
  }
  if (fs.statSync(file).size > 1024 * 1024) throw new Error("development manifest exceeds 1 MiB");
  const workspace = JSON.parse(fs.readFileSync(file, "utf8"));
  if (workspace.version !== 1 || !path.isAbsolute(workspace.worktree) || physicalPath(workspace.worktree) !== physicalPath(root)) {
    throw new Error("development manifest identity is inconsistent; run dev enable");
  }
  for (const key of ["systemBinary", "systemHome"]) {
    if (typeof workspace[key] !== "string" || !path.isAbsolute(workspace[key]) || workspace[key].split(path.sep).includes(".askhuman-dev")) {
      throw new Error("development manifest contains an invalid formal target");
    }
  }
  const expectedHome = path.join(os.homedir(), ".askhuman");
  const physical = (value) => { try { return physicalPath(value); } catch { return pathKey(path.resolve(value)); } };
  if (physical(workspace.systemHome) !== physical(expectedHome)) throw new Error("development manifest formal home is inconsistent");
  return workspace;
}

export function workerEnvironment(environment = process.env) {
  const result = { ...environment };
  for (const key of Object.keys(result)) {
    if (/^(ASKHUMAN_|ANTHROPIC_|OPENAI_|XAI_|CLAUDE_|CODEX_|GROK_|CURSOR_|PI_)/i.test(key)
        || ["CLAUDECODE", "TERM_SESSION_ID"].includes(key)) delete result[key];
  }
  result.ASKHUMAN_NO_KEYCHAIN = "1";
  return result;
}

export function capture(program, args, { root, signal, timeout, onCancel, env = workerEnvironment() } = {}) {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) return reject(new Error("development verification cancelled"));
    const child = spawn(program, args, { cwd: root, env, stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
    let stdout = Buffer.alloc(0), stderr = Buffer.alloc(0), failure, killTimer;
    const stop = (error) => {
      failure ??= error;
      child.kill();
      killTimer ??= setTimeout(() => child.kill("SIGKILL"), 1000);
      killTimer.unref();
    };
    const abort = () => {
      try { onCancel?.(); } catch { /* The socket still has to close even if retaining interruption fails. */ }
      stop(new Error("development verification cancelled"));
    };
    signal?.addEventListener("abort", abort, { once: true });
    const timer = timeout && setTimeout(() => stop(new Error("development capability probe timed out")), timeout);
    child.stdout.on("data", (data) => {
      if (stdout.length + data.length > maxOutput) stop(new Error("development result exceeds the output budget"));
      else stdout = Buffer.concat([stdout, data]);
    });
    child.stderr.on("data", (data) => {
      if (stderr.length + data.length > maxOutput) stop(new Error("development diagnostics exceed the output budget"));
      else stderr = Buffer.concat([stderr, data]);
    });
    function finish() {
      if (timer) clearTimeout(timer);
      if (killTimer) clearTimeout(killTimer);
      signal?.removeEventListener("abort", abort);
    }
    child.once("error", (error) => { finish(); reject(error); });
    child.once("close", (code) => {
      finish();
      if (failure) reject(failure);
      else resolve({ code, stdout: stdout.toString("utf8"), stderr: stderr.toString("utf8") });
    });
  });
}

export function askArguments(input) {
  const args = [];
  if (input.message?.trim()) args.push(input.message);
  for (const question of input.questions ?? []) {
    args.push("-q", question.question);
    for (const option of question.options ?? []) args.push(option.recommended ? "-o!" : "-o", option.text);
  }
  for (const file of input.files ?? []) args.push("-f", file);
  return args;
}

export function worker(root) {
  readWorkspace(root);
  const binary = path.join(root, ".askhuman-dev", "bin", process.platform === "win32" ? "AskHuman.exe" : "AskHuman");
  let actual;
  try { actual = fs.realpathSync(binary); fs.accessSync(binary, fs.constants.X_OK); }
  catch { throw new Error(`Dev binary is missing or not executable; run this worktree's install script: ${binary}`); }
  const relative = path.relative(physicalPath(path.join(root, ".askhuman-dev")), physicalPath(actual));
  if (!relative || relative.startsWith(`..${path.sep}`) || relative === ".." || path.isAbsolute(relative)) {
    throw new Error("Dev binary escapes this worktree's private directory");
  }
  return binary;
}

export async function runWorker(root, args, options = {}) {
  const binary = worker(root);
  const probe = await capture(binary, ["--development-capabilities"], { root, timeout: 3000, signal: options.signal });
  let capabilities;
  try { capabilities = JSON.parse(probe.stdout); } catch { capabilities = {}; }
  if (probe.code !== 0 || capabilities.developmentVerification !== 2) {
    throw new Error("installed worktree program does not support isolated development verification; rebuild it");
  }
  return capture(binary, args, { root, signal: options.signal, onCancel: options.onCancel, env: { ...workerEnvironment(), ASKHUMAN_DEV_DRIVER_PID: String(process.pid) } });
}

export async function runAsk(root, input, signal) {
  if (input.worktree && worktreeRoot(input.worktree) !== root) {
    throw new Error("this project MCP connection cannot verify another worktree");
  }
  const id = input.resume ?? randomUUID();
  if (!/^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/.test(id)) {
    throw new Error("verification ID must be a canonical UUID");
  }
  const args = ["dev", "ask", "--worktree", root];
  if (input.delivery) {
    if (!["local", "remote", "channels"].includes(input.delivery)) throw new Error("unsupported development delivery");
    args.push("--delivery", input.delivery);
  }
  if (input.resume) args.push("--resume", id);
  else {
    const question = askArguments(input);
    if (!question.length) throw new Error("dev_ask requires a message, questions, or a resume ID");
    args.push("--verification-id", id, "--", ...question);
  }
  const result = await runWorker(root, args, { signal, onCancel: () => {
    // Persist interruption before closing the socket, so a racing late answer cannot win.
    const result = spawnSync(worker(root), ["__dev-interrupt", root, id], { cwd: root, env: workerEnvironment(), timeout: 3000, stdio: "pipe", windowsHide: true });
    if (result.status !== 0) process.stderr.write("AskHuman: could not retain development cancellation; no answer is returned.\n");
  } });
  return { ...result, id };
}

export function record(root, id) {
  const directory = path.join(root, ".askhuman-dev", "home", "state", "verifications");
  const file = path.join(directory, `${id}.json`);
  const privateDirectory = physicalPath(path.join(root, ".askhuman-dev"));
  const relative = path.relative(privateDirectory, physicalPath(file));
  if (!relative || relative.startsWith("..") || path.isAbsolute(relative) || fs.statSync(file).size > maxOutput * 2) {
    throw new Error("verification result escapes the private worktree or exceeds its budget");
  }
  const value = JSON.parse(fs.readFileSync(file, "utf8"));
  if (value.info?.id !== id || physicalPath(value.info.worktree) !== physicalPath(root)) throw new Error("verification result identity mismatch");
  return { value, directory };
}

export function systemDefaultHome() {
  return path.join(os.homedir(), ".askhuman");
}
