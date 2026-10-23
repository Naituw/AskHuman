import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  chmodSync, closeSync, copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync,
  openSync, readFileSync, readdirSync, readlinkSync, rmSync, statSync, symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

const installer = fileURLToPath(new URL("./install.sh", import.meta.url));
const oldBinary = "#!/bin/sh\n# old binary\nexit 0\n";
const newBinary = "#!/bin/sh\n# new binary\nexit 0\n";
const unixOnly = { skip: process.platform === "win32" };

function executable(file, source) {
  writeFileSync(file, source);
  chmodSync(file, 0o755);
}

function fixture(t, { linked = false, fresh = false, os = "Darwin" } = {}) {
  const root = mkdtempSync(path.join(tmpdir(), "askhuman-install-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const repo = path.join(root, "repo");
  const bin = path.join(root, "installed bin");
  const stubs = path.join(root, "stubs");
  const targetDir = linked ? path.join(root, "npm package") : bin;
  for (const dir of [path.join(repo, "scripts"),
    path.join(repo, "src-tauri/target/local-install"), bin, stubs, targetDir]) {
    mkdirSync(dir, { recursive: true });
  }
  copyFileSync(installer, path.join(repo, "scripts/install.sh"));
  writeFileSync(path.join(repo, "scripts/build-frontend-if-needed.mjs"), "");
  executable(path.join(repo, "src-tauri/target/local-install/AskHuman"), newBinary);
  const target = path.join(targetDir, "AskHuman");
  const entry = path.join(bin, "AskHuman");
  if (!fresh) executable(target, oldBinary);
  if (linked) {
    const links = path.join(root, "links");
    mkdirSync(links);
    symlinkSync(target, path.join(links, "current"));
    symlinkSync("../links/current", entry);
  }
  const oldCopy = path.join(root, "old-copy");
  if (!fresh) writeFileSync(oldCopy, oldBinary);
  const state = path.join(bin, ".askhuman-install-state");
  if (!fresh) writeFileSync(state, "source=previous\ninstalled=previous\n");

  // Exercise the real installer without builds, keychain access, or production processes.
  for (const name of ["pnpm", "cargo", "cargo-sweep", "xattr"]) {
    executable(path.join(stubs, name), "#!/bin/sh\nexit 0\n");
  }
  symlinkSync(process.execPath, path.join(stubs, "node"));
  executable(path.join(stubs, "uname"), '#!/bin/sh\nprintf "%s\\n" "$TEST_OS"\n');
  executable(path.join(stubs, "codesign"), `#!/bin/sh
set -eu
for target; do :; done
[ "$target" != "$TEST_INSTALL_TARGET" ]
if [ -f "$TEST_OLD_COPY" ]; then
  cmp "$TEST_INSTALL_TARGET" "$TEST_OLD_COPY"
else
  [ ! -e "$TEST_INSTALL_TARGET" ]
fi
if [ "$1" = "--verify" ]; then
  [ "\${TEST_FAIL_VERIFY:-0}" != "1" ]
else
  [ "\${TEST_FAIL_SIGN:-0}" != "1" ]
  printf '# signed\\n' >> "$target"
fi
`);
  return {
    bin, target, targetDir, entry, state,
    run(extraEnv = {}) {
      return spawnSync("/bin/bash", [path.join(repo, "scripts/install.sh")], {
        cwd: repo,
        encoding: "utf8",
        timeout: 10_000,
        env: {
          ...process.env,
          PATH: `${stubs}:${bin}:/usr/bin:/bin:/usr/sbin:/sbin`,
          INSTALL_DIR: bin,
          CODESIGN_IDENTITY: "-",
          TEST_OS: os,
          TEST_INSTALL_TARGET: target,
          TEST_OLD_COPY: oldCopy,
          ...extraEnv,
        },
      });
    },
  };
}

function assertSuccess(result) {
  assert.equal(result.error, undefined);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
}

function assertNoStage(f) {
  assert.deepEqual(readdirSync(f.targetDir).filter(name => name.startsWith("AskHuman.new.")), []);
}

for (const os of ["Darwin", "Linux"]) {
  test(`${os}: publishing preserves the old open inode and unchanged installs skip replacement`, unixOnly, t => {
    const f = fixture(t, { os });
    const fd = openSync(f.target, "r");
    try {
      const oldInode = statSync(f.target).ino;
      assertSuccess(f.run());
      assert.notEqual(statSync(f.target).ino, oldInode);
      assert.equal(readFileSync(fd, "utf8"), oldBinary);
      const published = newBinary + (os === "Darwin" ? "# signed\n" : "");
      assert.equal(readFileSync(f.target, "utf8"), published);
      assert.equal(statSync(f.target).mode & 0o777, 0o755);
      assert.match(readFileSync(f.state, "utf8"),
        new RegExp(`installed=${createHash("sha256").update(published).digest("hex")}`));
      assertNoStage(f);

      const installedInode = statSync(f.target).ino;
      const second = f.run({ TEST_FAIL_SIGN: "1" });
      assertSuccess(second);
      assert.match(second.stdout, /跳过复制与签名/);
      assert.equal(statSync(f.target).ino, installedInode);
    } finally {
      closeSync(fd);
    }
  });
}

test("preserves a relative/absolute symlink chain and replaces its final target", unixOnly, t => {
  const f = fixture(t, { linked: true });
  const oldInode = statSync(f.target).ino;
  assertSuccess(f.run());
  assert.equal(lstatSync(f.entry).isSymbolicLink(), true);
  assert.equal(readlinkSync(f.entry), "../links/current");
  assert.notEqual(statSync(f.target).ino, oldInode);
  assert.equal(readFileSync(f.entry, "utf8"), newBinary + "# signed\n");
  assertNoStage(f);
});

for (const failure of ["TEST_FAIL_SIGN", "TEST_FAIL_VERIFY"]) {
  test(`${failure}: retains the installed binary and state, and removes staging`, unixOnly, t => {
    const f = fixture(t);
    const oldInode = statSync(f.target).ino;
    const result = f.run({ [failure]: "1" });
    assert.equal(result.error, undefined);
    assert.notEqual(result.status, 0);
    assert.equal(statSync(f.target).ino, oldInode);
    assert.equal(readFileSync(f.target, "utf8"), oldBinary);
    assert.equal(readFileSync(f.state, "utf8"), "source=previous\ninstalled=previous\n");
    assertNoStage(f);
  });
}

test("a first install publishes only after successful verification", unixOnly, t => {
  const f = fixture(t, { fresh: true });
  assert.notEqual(f.run({ TEST_FAIL_VERIFY: "1" }).status, 0);
  assert.equal(existsSync(f.target), false);
  assert.equal(existsSync(f.state), false);
  assertNoStage(f);
  assertSuccess(f.run());
  assert.equal(readFileSync(f.target, "utf8"), newBinary + "# signed\n");
  assertNoStage(f);
});
