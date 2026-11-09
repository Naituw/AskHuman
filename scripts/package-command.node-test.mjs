import assert from "node:assert/strict";
import test from "node:test";

import { pnpmScriptCommand } from "./package-command.mjs";

test("uses pnpm directly outside Windows", () => {
  assert.deepEqual(pnpmScriptCommand("build", { platform: "linux", env: {} }), {
    command: "pnpm",
    args: ["run", "build"],
  });
});

test("runs the Windows command shim through cmd.exe", () => {
  assert.deepEqual(
    pnpmScriptCommand("typecheck", {
      platform: "win32",
      env: { ComSpec: "C:\\Windows\\System32\\cmd.exe" },
    }),
    {
      command: "C:\\Windows\\System32\\cmd.exe",
      args: ["/d", "/s", "/c", "pnpm.cmd run typecheck"],
    },
  );
});

test("falls back to cmd.exe when ComSpec is absent", () => {
  assert.equal(
    pnpmScriptCommand("build", { platform: "win32", env: {} }).command,
    "cmd.exe",
  );
});
