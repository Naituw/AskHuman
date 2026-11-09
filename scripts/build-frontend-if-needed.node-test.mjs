import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import test from "node:test";

test("checks test-only changes without rebuilding the frontend bundle", (t) => {
  const root = mkdtempSync(join(tmpdir(), "askhuman-frontend-cache-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const dir of ["bin", "public", "scripts", "src/public"]) {
    mkdirSync(join(root, dir), { recursive: true });
  }

  const script = "build-frontend-if-needed.mjs";
  for (const name of [script, "package-command.mjs"]) {
    copyFileSync(new URL(`./${name}`, import.meta.url), join(root, "scripts", name));
  }
  const stub = join(root, "bin", "pnpm");
  writeFileSync(stub, `#!/usr/bin/env node
const { mkdirSync, writeFileSync } = require("node:fs");
const [command, script] = process.argv.slice(2);
if (command !== "run") process.exit(99);
console.log("pnpm:" + script);
if (script === "typecheck") process.exit(process.env.FAIL_TYPECHECK ? 1 : 0);
if (script !== "build") process.exit(99);
mkdirSync("dist", { recursive: true });
writeFileSync("dist/index.html", "bundle");
`, { mode: 0o755 });
  if (process.platform === "win32") {
    writeFileSync(`${stub}.cmd`, '@echo off\r\n"%NODE_EXE%" "%~dp0pnpm" %*\r\n');
  }

  const target = join(root, "target");
  const stamp = join(target, "frontend-build-cache.json");
  const write = (path, content) => writeFileSync(join(root, path), content);
  const run = (expected, { fail = false, args = [] } = {}) => {
    const result = spawnSync(process.execPath, [join(root, "scripts", script), ...args], {
      cwd: root,
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: [join(root, "bin"), process.env.PATH].join(delimiter),
        NODE_EXE: process.execPath,
        CARGO_TARGET_DIR: target,
        FAIL_TYPECHECK: fail ? "1" : "",
      },
    });
    assert.ifError(result.error);
    assert.equal(result.status, fail ? 1 : 0, result.stderr);
    assert.deepEqual(result.stdout.match(/^pnpm:.*$/gm) ?? [], expected ? [`pnpm:${expected}`] : []);
  };

  write("src/app.ts", "production");
  write("src/app.test.ts", "original test");
  run("build");
  run(null);

  const before = readFileSync(stamp, "utf8");
  write("src/app.test.ts", "changed test");
  run("typecheck", { fail: true });
  assert.equal(readFileSync(stamp, "utf8"), before);
  run("typecheck");
  run(null);
  assert.equal(readFileSync(join(root, "dist", "index.html"), "utf8"), "bundle");
  assert.equal(JSON.parse(readFileSync(stamp, "utf8")).outputDigest, JSON.parse(before).outputDigest);

  write("src/test-helper.testkit.ts", "test helper");
  run("typecheck");
  rmSync(join(root, "src/app.test.ts"));
  run("typecheck");

  for (const file of [
    "vite.attachment.config.ts",
    "scripts/vite-mermaid.ts",
    "public/attachment.css",
    "src/public/example.test.js",
    "src/app.ts",
    "dist/index.html",
  ]) {
    write(file, "changed");
    run("build");
    run(null);
  }
  run("build", { args: ["--force"] });
});
