// Real-process check of menu-bar spec D16: the GUI Host of an isolated instance leaves with its
// daemon instead of staying behind as an orphan menu-bar icon.
//
//   node scripts/gui-host-instance-lifecycle-smoke.mjs [candidate-binary]
//
// Runs private real daemons with a private ASKHUMAN_HOME and a private copy of the binary, and
// leaves `menuBarIcon` at its default (`always`): the setting that used to leave a resident Host
// behind after `daemon stop` and to re-exec it on every binary replacement. No model, bot or
// window is involved, but a daemon only spawns a GUI Host in a graphical session, so without one
// this prints SKIP and exits 0 (CI has no display and does not run it). While it runs, the private
// instance's icon shows up in the menu bar for about a minute.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";

if (process.platform === "win32") {
  console.log("SKIP: verify the Windows GUI Host lifecycle natively");
  process.exit(0);
}
const candidate = path.resolve(process.argv[2] ?? "src-tauri/target/local-install/AskHuman");
if (!fs.statSync(candidate, { throwIfNoEntry: false })?.isFile()) {
  // install.sh trims the build cache after installing, so pass the installed binary then.
  console.error(`candidate binary not found: ${candidate}\nusage: node scripts/gui-host-instance-lifecycle-smoke.mjs [binary, e.g. ~/.local/bin/AskHuman]`);
  process.exit(2);
}
const root = fs.realpathSync(fs.mkdtempSync(path.join(process.platform === "darwin" ? "/tmp" : os.tmpdir(), "ah-guihost-")));
const binary = path.join(root, "AskHuman"), cli = path.join(root, "AskHuman-cli");
const account = path.join(root, "user"), ah = path.join(root, "ah");
fs.mkdirSync(account); fs.mkdirSync(ah);
for (const target of [binary, cli]) { fs.copyFileSync(candidate, target, fs.constants.COPYFILE_FICLONE); fs.chmodSync(target, 0o700); }
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(ASKHUMAN_|CODEX_|CLAUDE_|CURSOR_|GROK_|PI_SESSION|PI_CODING)/.test(key)) delete env[key];
Object.assign(env, { HOME: account, ASKHUMAN_HOME: ah, ASKHUMAN_NO_KEYCHAIN: "1", GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: path.join(root, "gitconfig") });
fs.writeFileSync(env.GIT_CONFIG_GLOBAL, "");
// `menuBarIcon` is deliberately absent so that this fresh isolated home gets the default `always`.
fs.writeFileSync(path.join(ah, "config.json"), JSON.stringify({ general: { popupPrewarm: false }, channels: { autoActivation: false, popup: { enabled: false } } }));

const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function waitFor(check, ms) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) { if (check()) return true; await pause(200); }
  return check();
}
// Every Host of this run is launched from the private copy, so its command line is unique.
const hostPattern = `${path.basename(root)}/AskHuman --gui-host`;
function hostPids() {
  const result = spawnSync("pgrep", ["-f", hostPattern], { encoding: "utf8" });
  return (result.stdout ?? "").split("\n").filter(Boolean).map(Number);
}
let daemon;
function startDaemon() {
  const log = fs.openSync(path.join(root, "daemon.log"), "a");
  daemon = spawn(binary, ["daemon", "run"], { cwd: root, env, stdio: ["ignore", log, log] });
  fs.closeSync(log);
}
// The stop command comes from the untouched copy, whose fingerprint matches the running daemon,
// so it can never be mistaken for a client of a newer build.
function stopDaemon() {
  spawnSync(cli, ["daemon", "stop"], { cwd: root, env, encoding: "utf8", timeout: 20000 });
}
let revision = 0;
function replaceBinary() {
  const next = `${binary}.new`;
  fs.copyFileSync(candidate, next, fs.constants.COPYFILE_FICLONE); fs.chmodSync(next, 0o700);
  if (process.platform === "darwin") spawnSync("/usr/bin/codesign", ["--force", "--sign", "-", "--identifier", `com.askhuman.guihost-fixture.${++revision}`, next], { stdio: "ignore" });
  else fs.appendFileSync(next, `\nfixture revision ${++revision}\n`);
  fs.renameSync(next, binary);
}
async function startAndWaitForHost() {
  startDaemon();
  if (!await waitFor(() => fs.existsSync(path.join(ah, "daemon.json")), 30000)) throw new Error("the private daemon never started");
  return waitFor(() => hostPids().length > 0, 25000);
}

let failed = false;
try {
  // 1. The Host follows its daemon. It stays while the daemon is up, past its 12 s start-up grace
  //    (so it is the daemon, not the grace period, that holds it), and leaves once the daemon is gone.
  if (!await startAndWaitForHost()) {
    console.log("SKIP: the daemon spawned no GUI Host (no graphical session?)");
  } else {
    const first = hostPids();
    assert.equal(first.length, 1, `one GUI Host per instance, got ${first}`);
    await pause(14000);
    assert.deepEqual(hostPids(), first, "the Host must stay while its daemon is up");
    const stoppedAt = Date.now();
    stopDaemon();
    assert.ok(await waitFor(() => hostPids().length === 0, 30000), `a GUI Host stayed behind after its daemon stopped: ${hostPids()}`);
    console.log(`ok: the Host left ${Math.round((Date.now() - stoppedAt) / 1000)}s after its daemon stopped`);

    // 2. A binary replacement is no lease on life: replace the binary while the Host is up, then stop
    //    the daemon. Whether the Host re-execs first or not, nothing may be left behind.
    assert.ok(await startAndWaitForHost(), "the second daemon spawned no GUI Host");
    const before = hostPids();
    replaceBinary();
    stopDaemon();
    assert.ok(await waitFor(() => hostPids().length === 0, 70000), `a GUI Host survived the binary replacement: ${before} -> ${hostPids()}`);
    console.log("ok: no Host survived a binary replacement with its daemon gone");
  }
} catch (error) {
  failed = true;
  console.error(`FAIL: ${error.message}\nevidence: ${root}`);
} finally {
  stopDaemon();
  for (const pid of hostPids()) { try { process.kill(pid, "SIGKILL"); } catch {} }
  if (!failed) fs.rmSync(root, { recursive: true, force: true });
}
process.exit(failed ? 1 : 0);
