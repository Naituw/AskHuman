import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export const fixtureFlag = "--internal-system-fixture-v2";
export const targetEnvironment = "ASKHUMAN_RESOLVED_TARGET_V1";
let fixture;

export function accountHome() {
  const home = os.userInfo().homedir;
  if (!path.isAbsolute(home)) throw new Error("cannot resolve the operating system account home");
  return home;
}

function physical(value) {
  try { return fs.realpathSync.native(value); } catch { return path.resolve(value); }
}

export function configureFixture(value) {
  if (!path.isAbsolute(value)) throw new Error("System fixture account must be absolute");
  const home = fs.realpathSync.native(value);
  const stat = fs.statSync(home);
  if (!stat.isDirectory()) throw new Error("System fixture account must be a directory");
  if (process.platform !== "win32" && (stat.uid !== process.getuid() || (stat.mode & 0o022))) {
    throw new Error("System fixture account must be owned by this user and private from other writers");
  }
  const account = physical(accountHome()), root = path.join(home, ".askhuman");
  const production = physical(path.join(account, ".askhuman"));
  const within = (child, parent) => { const relative = path.relative(parent, child); return !relative || (!relative.startsWith("..") && !path.isAbsolute(relative)); };
  if (physical(home) === account || within(account, home) || within(home, production) || within(production, root)) {
    throw new Error("System fixture conflicts with the real account or production data");
  }
  try {
    const entry = fs.lstatSync(root);
    if (entry.isSymbolicLink() || !entry.isDirectory() || physical(root) !== root) {
      throw new Error("System fixture data directory must not redirect outside its account");
    }
  } catch (error) { if (error.code !== "ENOENT") throw error; }
  const selected = { version: 2, accountHome: home };
  if (process.env[targetEnvironment]) {
    const inherited = JSON.parse(process.env[targetEnvironment]);
    if (inherited.version !== 2 || inherited.fixture?.version !== 2
        || physical(inherited.fixture.accountHome) !== home) {
      throw new Error("explicit System fixture conflicts with the inherited target");
    }
  }
  if (fixture && fixture.accountHome !== home) throw new Error("System fixture context is already fixed");
  fixture = selected;
  return fixture;
}

export function fixtureContext() { return fixture; }
export function systemHome() { return path.join(fixture?.accountHome ?? accountHome(), ".askhuman"); }

export function devTarget(root, binary) {
  return {
    version: fixture ? 2 : 1, kind: "dev", binary,
    home: path.join(root, ".askhuman-dev/home"), worktree: root,
    reason: "project development controller", noKeychain: true,
    ...(fixture ? { fixture } : {}),
  };
}
