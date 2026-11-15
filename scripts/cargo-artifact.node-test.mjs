import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("./cargo-artifact.mjs", import.meta.url));

function workspace(t) {
  const root = mkdtempSync(path.join(tmpdir(), "askhuman-cargo-artifact-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
}

function binary(root, ...parts) {
  const file = path.join(root, ...parts);
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, "#!/bin/sh\nexit 0\n", { mode: 0o755 });
  return file;
}

function artifact(name, kind, executable, fresh = false) {
  return JSON.stringify({
    reason: "compiler-artifact",
    package_id: "humaninloop 0.0.0 (path+file:///src-tauri)",
    target: { kind, name },
    executable,
    fresh,
  });
}

function run(root, lines, name = "AskHuman") {
  const messages = path.join(root, "messages.json");
  writeFileSync(messages, lines.join("\n") + "\n");
  return spawnSync(process.execPath, [script, messages, name], { encoding: "utf8" });
}

test("reports the executable of the binary target and ignores every other message", t => {
  const root = workspace(t);
  const built = binary(root, "target", "local-install", "AskHuman");
  const result = run(root, [
    "warning: not json at all",
    "",
    artifact("humaninloop", ["lib"], null, true),
    // Same name, other kinds: never the installable binary.
    artifact("AskHuman", ["lib"], null),
    artifact("AskHuman", ["example"], binary(root, "target", "examples", "AskHuman")),
    artifact("other-tool", ["bin"], binary(root, "target", "local-install", "other-tool")),
    artifact("AskHuman", ["bin"], built),
    JSON.stringify({ reason: "build-finished", success: true }),
  ]);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, `${built}\n`);
});

test("a build that found everything up to date reports its artifact, whatever the file's age", t => {
  const root = workspace(t);
  const built = binary(root, "moved-target", "local-install", "AskHuman");
  // cargo leaves the file untouched when nothing changed, so it can be arbitrarily old.
  utimesSync(built, new Date(0), new Date(0));
  const result = run(root, [artifact("AskHuman", ["bin"], built, true)]);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, `${built}\n`);
});

test("the last report of the target wins", t => {
  const root = workspace(t);
  const first = binary(root, "a", "AskHuman");
  const second = binary(root, "b", "AskHuman");
  const result = run(root, [
    artifact("AskHuman", ["bin"], first),
    artifact("AskHuman", ["bin"], second),
  ]);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, `${second}\n`);
});

test("fails when cargo reported no executable for the target", t => {
  const root = workspace(t);
  const result = run(root, [
    artifact("humaninloop", ["lib"], null),
    artifact("other-tool", ["bin"], binary(root, "target", "other-tool")),
    JSON.stringify({ reason: "build-finished", success: true }),
  ]);
  assert.equal(result.status, 1);
  assert.equal(result.stdout, "");
  assert.match(result.stderr, /cargo 没有报告 AskHuman 的可执行文件/);
});

test("fails when the reported file does not exist", t => {
  const root = workspace(t);
  const missing = path.join(root, "target", "local-install", "AskHuman");
  const result = run(root, [artifact("AskHuman", ["bin"], missing)]);
  assert.equal(result.status, 1);
  assert.equal(result.stdout, "");
  assert.ok(result.stderr.includes(`cargo 报告的编译产物不存在: ${missing}`), result.stderr);
});

test("a missing argument is a usage error", () => {
  const result = spawnSync(process.execPath, [script], { encoding: "utf8" });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /Usage: cargo-artifact\.mjs/);
});
