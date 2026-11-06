#!/usr/bin/env node
// Verify npm and legacy binary-only direct delivery using a real compiled resource. No helper is
// launched or registered; resource-verify only unpacks into a temporary directory and checks trust.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const args = process.argv.slice(2);
const value = (key) => {
  const index = args.indexOf(key);
  assert.ok(index >= 0 && args[index + 1], `missing ${key}`);
  return resolve(args[index + 1]);
};
assert.equal(process.platform, "darwin", "the component trust check requires macOS");
const binary = value("--binary");
const archive = value("--archive");
const expectUnnotarized = args.includes("--expect-unnotarized");
const archIndex = args.indexOf("--arch");
const architecture = archIndex >= 0 ? args[archIndex + 1] : process.arch;
assert.ok(["arm64", "x64"].includes(architecture), "--arch must be arm64 or x64");
const root = fileURLToPath(new URL("..", import.meta.url));
const temporary = mkdtempSync(join(tmpdir(), "askhuman-resource-delivery-"));
const home = join(temporary, "home");
const state = join(temporary, "state");
mkdirSync(home);
const environment = { ...process.env, HOME: home, ASKHUMAN_HOME: state };
for (const key of Object.keys(environment)) {
  if (key.startsWith("ASKHUMAN_DEV_") || key === "ASKHUMAN_RESOLVED_TARGET_V1") delete environment[key];
}
const run = (program, argv, options = {}) => {
  const result = spawnSync(program, argv, { env: environment, encoding: "utf8", ...options });
  assert.equal(result.status, 0, `${program}: ${result.error?.message ?? result.stderr}`);
  return result.stdout;
};
const expected = createHash("sha256").update(readFileSync(archive)).digest("hex");
const check = (program) => {
  const info = JSON.parse(run(program, ["todo-sync", "resource-info", "--require-bundled"]));
  assert.equal(info.bundled, true);
  assert.equal(info.sha256, expected);
  const verified = spawnSync(program, ["todo-sync", "resource-verify"], { env: environment, encoding: "utf8" });
  if (expectUnnotarized) {
    assert.equal(verified.status, 1, "an unnotarized resource must fail trust validation");
    assert.match(verified.stderr, /code failed to satisfy specified code requirement/);
  } else {
    assert.equal(verified.status, 0, verified.stderr);
  }
  return info;
};

try {
  const outerArchitectures = run("/usr/bin/lipo", ["-archs", binary]).trim().split(/\s+/);
  assert.ok(outerArchitectures.includes(architecture === "arm64" ? "arm64" : "x86_64"), "the npm platform must match the compiled binary architecture");
  const original = check(binary);
  const manifest = join(root, "packaging/npm/platforms", architecture === "arm64" ? "darwin-arm64" : "darwin-x64", "package.json");
  const packageRoot = join(temporary, "npm-input");
  mkdirSync(join(packageRoot, "bin"), { recursive: true });
  copyFileSync(manifest, join(packageRoot, "package.json"));
  copyFileSync(binary, join(packageRoot, "bin/AskHuman"));
  const packed = JSON.parse(run("npm", ["pack", "--json", "--ignore-scripts", "--pack-destination", temporary], { cwd: packageRoot }));
  const npmExtract = join(temporary, "npm-extract");
  mkdirSync(npmExtract);
  run("/usr/bin/tar", ["-xzf", join(temporary, packed[0].filename), "-C", npmExtract]);
  assert.deepEqual(check(join(npmExtract, "package/bin/AskHuman")), original);

  // The released v0.16.0 updater discards everything except this one file after extracting.
  const download = join(temporary, "download.tar.gz");
  run("/usr/bin/tar", ["-czf", download, "-C", dirname(binary), "AskHuman"]);
  const directExtract = join(temporary, "direct-extract");
  const directInstall = join(temporary, "legacy-install");
  mkdirSync(directExtract); mkdirSync(directInstall);
  run("/usr/bin/tar", ["-xzf", download, "-C", directExtract]);
  copyFileSync(join(directExtract, "AskHuman"), join(directInstall, "AskHuman"));
  assert.deepEqual(check(join(directInstall, "AskHuman")), original);
  assert.equal(existsSync(join(home, "Applications")), false, "verification must not install an app");
  assert.equal(existsSync(join(state, "state/todo-sync")), false, "verification must not create sync state");
  assert.equal(existsSync(join(home, ".askhuman")), false, "verification must not write user state");
  console.log(JSON.stringify({ architecture, npmPack: true, legacyDirectCopy: true, resourceTrust: expectUnnotarized ? "unnotarized rejected" : "verified", noUserState: true, sha256: expected }));
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
