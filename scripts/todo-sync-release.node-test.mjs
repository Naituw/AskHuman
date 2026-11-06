import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

test("helper release rejects development profiles, wrong publishers and unapproved uploads", () => {
  const result = spawnSync(process.platform === "win32" ? "python" : "python3", [
    "-B", "-m", "unittest", "discover", "-s", "todo-sync-helper/scripts", "-p", "release_test.py",
  ], { cwd: fileURLToPath(new URL("..", import.meta.url)), encoding: "utf8" });
  assert.equal(result.status, 0, result.error?.message ?? result.stderr);
});

test("release resources survive compilation and cannot be omitted or embedded on other platforms", () => {
  const temporary = mkdtempSync(join(tmpdir(), "askhuman-resource-generator-"));
  try {
    const executable = join(temporary, process.platform === "win32" ? "tests.exe" : "tests");
    const source = fileURLToPath(new URL("../src-tauri/build/todo_sync_resource.rs", import.meta.url));
    const compiled = spawnSync("rustc", ["--edition", "2021", "--test", source, "-o", executable], { encoding: "utf8" });
    assert.equal(compiled.status, 0, compiled.error?.message ?? compiled.stderr);
    const result = spawnSync(executable, [], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr + result.stdout);
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
});
