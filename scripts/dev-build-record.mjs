import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

export const sourceDirectories = [
  "src", "public", "scripts", "src-tauri/src", "src-tauri/resources",
  "src-tauri/swift", "src-tauri/icons", "src-tauri/capabilities", "src-tauri/vendor",
];
export const sourceFiles = [
  "package.json", "pnpm-lock.yaml", "vite.config.ts", "vite.attachment.config.ts",
  "tsconfig.json", "src-tauri/Cargo.toml", "src-tauri/Cargo.lock",
  "src-tauri/tauri.conf.json", "src-tauri/build.rs", "src-tauri/Info.plist",
];

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

export function assertDevTarget(root, binary) {
  root = fs.realpathSync(root);
  const directory = fs.realpathSync(path.join(root, ".askhuman-dev"));
  function inside(parent, child) {
    const relative = path.relative(parent, child);
    return relative !== "" && !relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative);
  }
  if (!inside(root, directory)) throw new Error("Dev directory escapes the selected worktree");
  const targetDirectory = fs.realpathSync(path.dirname(binary));
  if (!inside(directory, targetDirectory)) throw new Error("Dev install target escapes the private instance directory");
  const instanceHome = fs.realpathSync(path.join(root, ".askhuman-dev", "home"));
  if (!inside(directory, instanceHome)) throw new Error("Dev home escapes the private instance directory");
  let target = binary;
  for (let hop = 0; hop < 40; hop++) {
    if (!inside(directory, path.join(fs.realpathSync(path.dirname(target)), path.basename(target)))) {
      throw new Error("Dev binary points outside its private instance directory");
    }
    let stat;
    try { stat = fs.lstatSync(target); }
    catch (error) { if (error.code === "ENOENT") return; throw error; }
    if (!stat.isSymbolicLink()) return;
    target = path.resolve(path.dirname(target), fs.readlinkSync(target));
  }
  throw new Error("Dev binary has too many symbolic links");
}

export function sourceFingerprint(root) {
  const inputs = [];
  function visit(relative) {
    const file = path.join(root, relative);
    let stat;
    try { stat = fs.lstatSync(file); }
    catch (error) { if (error.code === "ENOENT") return; throw error; }
    if (stat.isSymbolicLink()) inputs.push([relative, digest(Buffer.from(`link:${fs.readlinkSync(file)}`))]);
    else if (stat.isFile()) inputs.push([relative, digest(fs.readFileSync(file))]);
    else if (stat.isDirectory()) {
      for (const name of fs.readdirSync(file)) {
        if ([".git", "node_modules", "target", ".DS_Store"].includes(name)) continue;
        visit(`${relative}/${name}`);
      }
    }
  }
  for (const relative of [...sourceDirectories, ...sourceFiles]) visit(relative);
  inputs.sort(([a], [b]) => Buffer.compare(Buffer.from(a), Buffer.from(b)));
  const hash = createHash("sha256");
  for (const [relative, fingerprint] of inputs) hash.update(relative).update("\0").update(fingerprint).update("\0");
  return hash.digest("hex");
}

export function recordBuild(root, binary, before) {
  root = fs.realpathSync(root);
  assertDevTarget(root, binary);
  const record = path.join(root, ".askhuman-dev", "build.json");
  const after = sourceFingerprint(root);
  if (!/^[a-f0-9]{64}$/.test(before) || after !== before) {
    fs.rmSync(record, { force: true });
    throw new Error("Build inputs changed during installation; rebuild before Dev verification");
  }
  const git = spawnSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" });
  const value = {
    version: 1, worktree: root, binary: fs.realpathSync(binary),
    sourceFingerprint: after, binaryFingerprint: digest(fs.readFileSync(binary)),
    gitCommit: git.status === 0 ? git.stdout.trim() : null, createdAtMs: Date.now(),
  };
  fs.mkdirSync(path.dirname(record), { recursive: true, mode: 0o700 });
  const temporary = `${record}.${process.pid}.tmp`;
  try {
    fs.writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600, flag: "wx" });
    fs.renameSync(temporary, record);
  } finally { fs.rmSync(temporary, { force: true }); }
  return value;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const [operation, root, binary, before] = process.argv.slice(2);
    if (operation === "snapshot" && root) console.log(sourceFingerprint(root));
    else if (operation === "assert-target" && root && binary) assertDevTarget(root, binary);
    else if (operation === "record" && root && binary && before) console.log(JSON.stringify(recordBuild(root, binary, before)));
    else throw new Error("Usage: dev-build-record.mjs snapshot ROOT | record ROOT BINARY SOURCE_FINGERPRINT");
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
