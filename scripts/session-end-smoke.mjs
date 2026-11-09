// Sends Quit Apple Events only to private AskHuman popup processes on this Mac.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { createConnection } from "node:net";
import { createInterface } from "node:readline";

if (process.platform !== "darwin") {
  console.log("SKIP: requires macOS Apple Events and a desktop session");
  process.exit(0);
}
const source = path.resolve(process.argv[2] ?? "src-tauri/target/local-install/AskHuman");
const root = fs.realpathSync(fs.mkdtempSync(path.join("/tmp", "ah-session-")));
const environment = { ...process.env };
for (const key of Object.keys(environment)) {
  if (/^(ASKHUMAN_|CODEX_|CLAUDE_|CURSOR_|GROK_|PI_)/.test(key)) delete environment[key];
}
environment.ASKHUMAN_NO_KEYCHAIN = "1";
const children = [];
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

async function until(check, description) {
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch {}
    await sleep(100);
  }
  throw new Error(`${description} timed out, evidence ${root}`);
}

function status(data) {
  return new Promise((resolve, reject) => {
    const socket = createConnection(path.join(data, "daemon.sock"));
    const timeout = setTimeout(() => { socket.destroy(); reject(new Error("status timed out")); }, 1000);
    socket.on("error", error => { clearTimeout(timeout); reject(error); });
    createInterface({ input: socket }).on("error", () => {}).once("line", line => {
      clearTimeout(timeout); socket.destroy(); resolve(JSON.parse(line));
    });
    socket.on("connect", () => socket.write('{"type":"status"}\n'));
  });
}

function popupPid(binary, mode) {
  const processList = spawnSync("ps", ["-axo", "pid=,command="], { encoding: "utf8" });
  assert.equal(processList.status, 0, processList.stderr);
  const role = mode === "merged" ? "--popup-host" : "--popup";
  const row = processList.stdout.split("\n").find(line => line.includes(binary) && line.includes(role));
  return row && Number(row.trim().split(/\s+/)[0]);
}

const senderSource = path.join(root, "main.swift");
fs.writeFileSync(senderSource, `
import AppKit
import Carbon
let pid = pid_t(CommandLine.arguments[1])!
let expected = URL(fileURLWithPath: CommandLine.arguments[3]).resolvingSymlinksInPath()
guard let running = NSRunningApplication(processIdentifier: pid),
    running.executableURL?.resolvingSymlinksInPath().path == expected.path
else { fputs("target is not the private AskHuman executable\\n", stderr); exit(1) }
let reason = CommandLine.arguments[2].utf8.reduce(UInt32(0)) { ($0 << 8) | UInt32($1) }
let event = NSAppleEventDescriptor(eventClass: AEEventClass(kCoreEventClass), eventID: AEEventID(kAEQuitApplication),
    targetDescriptor: NSAppleEventDescriptor(processIdentifier: pid), returnID: AEReturnID(kAutoGenerateReturnID),
    transactionID: AETransactionID(kAnyTransactionID))
event.setParam(NSAppleEventDescriptor(enumCode: reason), forKeyword: AEKeyword(kAEQuitReason))
do { _ = try event.sendEvent(options: [.noReply, .neverInteract, .dontRecord], timeout: 2) }
catch { fputs("targeted Quit event failed: \\(error)\\n", stderr); exit(1) }
`);
const sender = path.join(root, "send-quit");
const compilation = spawnSync("xcrun", ["swiftc", senderSource, "-o", sender], { encoding: "utf8", timeout: 60000 });
assert.equal(compilation.status, 0, compilation.stderr || compilation.error?.message);

try {
  for (const [name, reason, mode] of [
    ["shutdown", "shut", "merged"],
    ["restart", "rest", "merged"],
    ["logout", "rlgo", "merged"],
    ["independent-shutdown", "shut", "independent"],
  ]) {
    const directory = path.join(root, name), account = path.join(directory, "user"), data = path.join(account, ".askhuman");
    fs.mkdirSync(data, { recursive: true });
    const binary = path.join(directory, "AskHuman");
    fs.copyFileSync(source, binary, fs.constants.COPYFILE_FICLONE); fs.chmodSync(binary, 0o700);
    fs.writeFileSync(path.join(data, "config.json"), JSON.stringify({
      general: { menuBarIcon: "off", popupPrewarm: false, alwaysOnTop: false },
      channels: { popup: { enabled: true, windowMode: mode }, autoActivation: false },
    }));
    const prefix = ["--development-capabilities", "--internal-system-fixture-v2", account, "--"];
    const env = { ...environment, HOME: path.join(directory, "caller-home") };
    const logPath = path.join(directory, "daemon.log"), log = fs.openSync(logPath, "a");
    const daemon = spawn(binary, [...prefix, "daemon", "run"], { cwd: directory, env, stdio: ["ignore", log, log] });
    fs.closeSync(log); children.push(daemon);
    await until(() => status(data), "private daemon startup");
    const caller = spawn(binary, [...prefix, "-q", `private ${name} regression`, "-o", "continue"], {
      cwd: directory, env, stdio: ["ignore", "pipe", "pipe"],
    });
    children.push(caller);
    let stdout = "", stderr = "";
    caller.stdout.on("data", data => stdout += data); caller.stderr.on("data", data => stderr += data);
    const result = new Promise(resolve => caller.once("close", code => resolve({ code, stdout, stderr })));
    await until(() => popupPid(binary, mode), "private popup startup");
    await sleep(3000);
    let popup = popupPid(binary, mode);
    assert.ok(popup);
    if (name === "shutdown") {
      process.kill(popup, "SIGKILL");
      await until(() => { const next = popupPid(binary, mode); return next && next !== popup; }, "ordinary crash recovery");
      await sleep(3000);
      popup = popupPid(binary, mode);
      assert.ok(popup); assert.equal(daemon.exitCode, null);
      console.log("PASS: an ordinary popup crash still restores its pending request");
    }
    const before = fs.readFileSync(logPath, "utf8").match(/popup host disconnected/g)?.length ?? 0;
    const sent = spawnSync(sender, [String(popup), reason, binary], { encoding: "utf8", timeout: 10000 });
    assert.equal(sent.status, 0, sent.stderr || sent.error?.message);
    await until(() => caller.exitCode !== null, "cancelled caller result");
    const completed = await result;
    assert.equal(completed.code, 3, completed.stderr);
    assert.match(completed.stdout + completed.stderr, /cancel|取消/i);
    await until(() => daemon.exitCode !== null, "silent instance shutdown");
    assert.equal(popupPid(binary, mode), undefined);
    const after = fs.readFileSync(logPath, "utf8");
    assert.match(after, /stop requested/);
    assert.ok((after.match(/popup host disconnected/g)?.length ?? 0) <= before + 1);
    console.log(`PASS: ${name} cancels the pending request and stops the instance without a replacement popup`);
  }
  console.log(`Evidence ${root}`);
} finally {
  for (const child of children) if (child.exitCode === null) child.kill();
}
