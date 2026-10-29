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

function git(cwd, ...args) {
  const result = spawnSync("git", ["-c", "user.name=test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", ...args],
    { cwd, encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
}

// "main": a plain git checkout. "worktree": a linked worktree of a separate main checkout.
function initCheckout(root, repo, kind) {
  if (kind === "main") {
    mkdirSync(repo, { recursive: true });
    git(repo, "init", "-q");
    return;
  }
  const main = path.join(root, "main");
  mkdirSync(main);
  git(main, "init", "-q");
  git(main, "commit", "-q", "--allow-empty", "-m", "init");
  git(main, "worktree", "add", "-q", repo);
}

// Replays scripted sizes for `du -sk <dir>`: calls for a matching directory return the `first`
// size once, then `after` (what Cargo's cleanup is expected to leave behind). Other paths use the
// real du so unrelated directories behave normally.
function duStub(rules) {
  const resolved = spawnSync("sh", ["-c", "command -v du"], { encoding: "utf8" });
  assert.equal(resolved.status, 0, resolved.stderr);
  const realDu = "'" + resolved.stdout.trim().replaceAll("'", "'\\''") + "'";
  const cases = rules.map(({ match, first, after }, i) => `*${match}*)
    n=$(cat "$TEST_DU_STATE.${i}" 2>/dev/null || echo 0)
    echo $((n + 1)) > "$TEST_DU_STATE.${i}"
    if [ "$n" -eq 0 ]; then size=${first}; else size=${after}; fi
    printf '%s\\t%s\\n' "$size" "$dir"
    exit 0 ;;`).join("\n");
  return `#!/bin/sh
dir="$2"
case "$dir" in
${cases}
esac
exec ${realDu} "$@"
`;
}

// Records every cargo invocation, the RUSTC_BOOTSTRAP value `cargo build` saw, and can replay
// cargo-sweep output.
const recordingCargo = `#!/bin/sh
printf '%s\\n' "$*" >> "$TEST_CARGO_LOG"
if [ "\${1:-}" = "build" ]; then printf '%s\\n' "\${RUSTC_BOOTSTRAP-<unset>}" > "$TEST_CARGO_BUILD_ENV"; fi
if [ "\${1:-}" = "sweep" ] && [ -n "\${TEST_SWEEP_OUTPUT:-}" ]; then printf '%s\\n' "$TEST_SWEEP_OUTPUT"; fi
exit 0
`;

const MIB = 1024; // du -sk reports KiB

// A RUSTC_BOOTSTRAP exported in the shell running the suite must not leak into the installer.
const inheritedEnv = (() => {
  const env = { ...process.env };
  delete env.RUSTC_BOOTSTRAP;
  return env;
})();

function fixture(t, { linked = false, fresh = false, os = "Darwin", checkout = "none", stubs: extraStubs = {} } = {}) {
  const root = mkdtempSync(path.join(tmpdir(), "askhuman-install-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const repo = path.join(root, "repo");
  const bin = path.join(root, "installed bin");
  const stubs = path.join(root, "stubs");
  const targetDir = linked ? path.join(root, "npm package") : bin;
  if (checkout !== "none") initCheckout(root, repo, checkout);
  for (const dir of [path.join(repo, "scripts"),
    path.join(repo, "src-tauri/target/local-install"), bin, stubs, targetDir]) {
    mkdirSync(dir, { recursive: true });
  }
  copyFileSync(installer, path.join(repo, "scripts/install.sh"));
  copyFileSync(new URL("./install-linux.rs", import.meta.url), path.join(repo, "scripts/install-linux.rs"));
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
  for (const [name, source] of Object.entries(extraStubs)) executable(path.join(stubs, name), source);
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
  const cacheLog = path.join(repo, "src-tauri/target/askhuman-cache.log");
  const cargoLog = path.join(root, "cargo.log");
  const cargoBuildEnv = path.join(root, "cargo-build-env");
  return {
    bin, target, targetDir, entry, state, repo, cacheLog,
    cargoCalls: () => (existsSync(cargoLog) ? readFileSync(cargoLog, "utf8").trim().split("\n") : []),
    cargoBuildEnv: () => readFileSync(cargoBuildEnv, "utf8").trim(),
    cacheLines: () => (existsSync(cacheLog) ? readFileSync(cacheLog, "utf8").trim().split("\n") : []),
    run(extraEnv = {}) {
      return spawnSync("bash", [path.join(repo, "scripts/install.sh")], {
        cwd: repo,
        encoding: "utf8",
        // The suite runs on busy developer machines where the installer alone can take seconds.
        timeout: 30_000,
        env: {
          ...inheritedEnv,
          PATH: [stubs, bin, inheritedEnv.PATH].join(path.delimiter),
          INSTALL_DIR: bin,
          CODESIGN_IDENTITY: "-",
          TEST_OS: os,
          TEST_INSTALL_TARGET: target,
          TEST_OLD_COPY: oldCopy,
          TEST_DU_STATE: path.join(root, "du-state"),
          TEST_CARGO_LOG: cargoLog,
          TEST_CARGO_BUILD_ENV: cargoBuildEnv,
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
  // The publisher uses Linux syscalls; Darwin's signing branch can be simulated on any Unix host.
  const nativePublisher = os === "Linux" && process.platform !== "linux";
  test(`${os}: publishing preserves the old open inode and unchanged installs skip replacement`,
    { skip: process.platform === "win32" || (nativePublisher && "requires the native Linux publisher") }, t => {
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
      assert.match(second.stdout, os === "Linux"
        ? /installed executable unchanged; skipping publication/
        : /跳过复制与签名/);
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

// --- target/ cache budgets, log and environment guard ---

const SHARED_CACHE = "src-tauri/target/swift-module-cache";

test("the main checkout keeps the standard 4 GiB local-install budget", unixOnly, t => {
  const f = fixture(t, {
    checkout: "main",
    stubs: {
      cargo: recordingCargo,
      du: duStub([{ match: "target/local-install", first: 3584 * MIB, after: 3584 * MIB }]),
    },
  });
  assertSuccess(f.run());
  assert.equal(f.cargoCalls().some(call => call.startsWith("clean")), false);
  assert.equal(f.cacheLines().some(line => line.includes("budget:")), false);
});

test("a linked worktree gets the tighter 3 GiB budget and the package cleanup is logged", unixOnly, t => {
  const f = fixture(t, {
    checkout: "worktree",
    stubs: {
      cargo: recordingCargo,
      du: duStub([{ match: "target/local-install", first: 3584 * MIB, after: 1000 * MIB }]),
    },
  });
  const result = f.run();
  assertSuccess(result);
  assert.deepEqual(f.cargoCalls().filter(call => call.startsWith("clean")),
    ["clean --manifest-path src-tauri/Cargo.toml -p humaninloop --profile local-install"]);
  assert.match(result.stdout, /超过预算 3072MB（linked worktree）/);
  assert.ok(f.cacheLines().some(line =>
    line.endsWith("budget: local-install 3584MB > 3072MB (linked worktree); package clean -> 1000MB")));
});

test("dependencies that alone exceed the budget escalate to a profile clean", unixOnly, t => {
  const f = fixture(t, {
    checkout: "worktree",
    stubs: {
      cargo: recordingCargo,
      du: duStub([{ match: "target/local-install", first: 3584 * MIB, after: 3300 * MIB }]),
    },
  });
  assertSuccess(f.run());
  assert.deepEqual(f.cargoCalls().filter(call => call.startsWith("clean")), [
    "clean --manifest-path src-tauri/Cargo.toml -p humaninloop --profile local-install",
    "clean --manifest-path src-tauri/Cargo.toml --profile local-install",
  ]);
  assert.ok(f.cacheLines().some(line =>
    line.endsWith("budget: local-install 3584MB > 3072MB (linked worktree); profile clean -> 3300MB")));
});

test("a source tree that is not a git checkout counts as the main checkout", unixOnly, t => {
  const f = fixture(t, {
    checkout: "none",
    stubs: {
      cargo: recordingCargo,
      du: duStub([{ match: "target/local-install", first: 3584 * MIB, after: 3584 * MIB }]),
    },
  });
  assertSuccess(f.run());
  assert.equal(f.cargoCalls().some(call => call.startsWith("clean")), false);
});

test("RUSTC_BOOTSTRAP from the calling shell is ignored for the build and logged", unixOnly, t => {
  const f = fixture(t, { stubs: { cargo: recordingCargo } });
  const result = f.run({ RUSTC_BOOTSTRAP: "1" });
  assertSuccess(result);
  assert.equal(f.cargoBuildEnv(), "<unset>");
  assert.match(result.stderr, /RUSTC_BOOTSTRAP=1/);
  assert.ok(f.cacheLines().some(line => line.endsWith("env: RUSTC_BOOTSTRAP=1 was set; unset for this build")));

  // Set but empty still changes the fingerprint of the build scripts that track it.
  const empty = fixture(t, { stubs: { cargo: recordingCargo } });
  assertSuccess(empty.run({ RUSTC_BOOTSTRAP: "" }));
  assert.equal(empty.cargoBuildEnv(), "<unset>");
  assert.ok(empty.cacheLines().some(line => line.includes("env: RUSTC_BOOTSTRAP= was set")));

  const clean = fixture(t, { stubs: { cargo: recordingCargo } });
  const quiet = clean.run();
  assertSuccess(quiet);
  assert.doesNotMatch(quiet.stderr, /RUSTC_BOOTSTRAP/);
  assert.equal(clean.cacheLines().some(line => line.includes("env:")), false);
});

test("the cache log keeps only its newest 200 lines", unixOnly, t => {
  const f = fixture(t, { stubs: { cargo: recordingCargo } });
  mkdirSync(path.dirname(f.cacheLog), { recursive: true });
  writeFileSync(f.cacheLog,
    Array.from({ length: 250 }, (_, i) => `2026-01-01T00:00:00+0000 old entry ${i + 1}\n`).join(""));
  assertSuccess(f.run({ RUSTC_BOOTSTRAP: "1" }));
  const lines = f.cacheLines();
  assert.equal(lines.length, 200);
  assert.equal(lines.some(line => line.endsWith("old entry 1")), false);
  assert.ok(lines.some(line => line.includes("env: RUSTC_BOOTSTRAP=1")));
});

test("the toolchain is logged when it first appears and whenever it changes", unixOnly, t => {
  const f = fixture(t, {
    stubs: {
      cargo: recordingCargo,
      rustc: '#!/bin/sh\necho "rustc $TEST_RUSTC_VERSION (test)"\n',
      xcrun: "#!/bin/sh\necho 99.0\n",
    },
  });
  const toolchainLines = () => f.cacheLines().filter(line => line.includes(" toolchain: "));
  assertSuccess(f.run({ TEST_RUSTC_VERSION: "1.0.0" }));
  assert.equal(toolchainLines().length, 1);
  assert.ok(toolchainLines()[0].endsWith("toolchain: rustc 1.0.0 (test) | macosx-sdk 99.0"));
  assertSuccess(f.run({ TEST_RUSTC_VERSION: "1.0.0" }));
  assert.equal(toolchainLines().length, 1);
  assertSuccess(f.run({ TEST_RUSTC_VERSION: "1.0.1" }));
  assert.equal(toolchainLines().length, 2);
  assert.ok(toolchainLines()[1].endsWith("toolchain: rustc 1.0.1 (test) | macosx-sdk 99.0"));
});

test("per-build Swift module caches from the old layout are removed, the shared one stays", unixOnly, t => {
  const f = fixture(t, { stubs: { cargo: recordingCargo } });
  const legacy = path.join(f.repo, "src-tauri/target/local-install/build/humaninloop-abc/out/swift-module-cache");
  const shared = path.join(f.repo, SHARED_CACHE);
  for (const dir of [legacy, shared]) {
    mkdirSync(dir, { recursive: true });
    writeFileSync(path.join(dir, "module.pcm"), "x");
  }
  assertSuccess(f.run());
  assert.equal(existsSync(legacy), false);
  assert.equal(existsSync(path.join(shared, "module.pcm")), true);
  assert.equal(f.cacheLines().filter(line => line.includes("migrate: removed 1 per-build")).length, 1);

  assertSuccess(f.run());
  assert.equal(f.cacheLines().filter(line => line.includes("migrate:")).length, 1);
});

test("an oversized shared Swift module cache is removed and logged", unixOnly, t => {
  const f = fixture(t, {
    stubs: {
      cargo: recordingCargo,
      du: duStub([{ match: "target/swift-module-cache", first: 900 * MIB, after: 900 * MIB }]),
    },
  });
  const shared = path.join(f.repo, SHARED_CACHE);
  mkdirSync(shared, { recursive: true });
  writeFileSync(path.join(shared, "module.pcm"), "x");
  assertSuccess(f.run());
  assert.equal(existsSync(shared), false);
  assert.ok(f.cacheLines().some(line =>
    line.endsWith("budget: swift-module-cache 900MB > 768MB; removed, rebuilt by the next build")));
});

test("cargo-sweep results are logged only when something was cleaned", unixOnly, t => {
  const nothing = fixture(t, { stubs: { cargo: recordingCargo } });
  assertSuccess(nothing.run({ TEST_SWEEP_OUTPUT: '[INFO] Cleaned nothing from "/x/target"' }));
  assert.equal(nothing.cacheLines().some(line => line.includes("sweep:")), false);

  const cleaned = fixture(t, { stubs: { cargo: recordingCargo } });
  assertSuccess(cleaned.run({ TEST_SWEEP_OUTPUT: '[INFO] Cleaned 1.50 GiB from "/x/target"' }));
  assert.ok(cleaned.cacheLines().some(line => line.endsWith("sweep: cargo sweep --time 7 cleaned 1.50 GiB")));
});
