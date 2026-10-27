import assert from "node:assert/strict";
import {
  chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync,
  rmSync, statSync, symlinkSync, writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

const seeder = fileURLToPath(new URL("./seed-worktree-target.sh", import.meta.url));
// Cloning needs a real APFS volume, so the end-to-end cases only run on macOS.
const darwinOnly = { skip: process.platform !== "darwin" };
const unixOnly = { skip: process.platform === "win32" };

function executable(file, source) {
  writeFileSync(file, source);
  chmodSync(file, 0o755);
}

function put(file, contents) {
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, contents);
}

function git(cwd, ...args) {
  const result = spawnSync("git", ["-c", "user.name=test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false", ...args],
    { cwd, encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
}

// Records every cargo call. `metadata` lists two path packages and one registry package;
// TEST_CARGO_METADATA=none makes it print nothing, TEST_CARGO_CLEAN_STATUS makes `clean` fail.
const cargoStub = `#!/bin/sh
printf '%s\\n' "$*" >> "$TEST_CARGO_LOG"
case "\${1:-}" in
  metadata)
    if [ "\${TEST_CARGO_METADATA:-ok}" = "ok" ]; then
      printf '%s' '{"packages":[{"name":"humaninloop","source":null},{"name":"drag","source":null},{"name":"serde","source":"registry+https://github.com/rust-lang/crates.io-index"}]}'
    fi ;;
  clean) exit "\${TEST_CARGO_CLEAN_STATUS:-0}" ;;
esac
exit 0
`;

// "main" is a git checkout holding build output; "wt" is a linked worktree of it, i.e. the new
// checkout that gets seeded.
function fixture(t, { os = "Darwin", stubs: extraStubs = {} } = {}) {
  const root = realpathSync(mkdtempSync(path.join(tmpdir(), "askhuman-seed-")));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const main = path.join(root, "main");
  const worktree = path.join(root, "wt");
  const stubs = path.join(root, "stubs");
  const cargoLog = path.join(root, "cargo.log");
  mkdirSync(main);
  mkdirSync(stubs);
  git(main, "init", "-q");
  git(main, "commit", "-q", "--allow-empty", "-m", "init");
  git(main, "worktree", "add", "-q", worktree);

  const mainTarget = path.join(main, "src-tauri/target");
  put(path.join(mainTarget, "local-install/deps/dep.rlib"), "local-install dependency");
  put(path.join(mainTarget, "local-install/build/humaninloop-abc123/out/swift-module-cache/legacy.pcm"),
    "module cache of the old per-build layout");
  put(path.join(mainTarget, "debug/deps/dep.rlib"), "debug dependency");
  put(path.join(mainTarget, "release/deps/dep.rlib"), "release dependency");
  put(path.join(mainTarget, "swift-module-cache/shared.pcm"), "shared module cache");
  for (const checkout of [main, worktree]) {
    mkdirSync(path.join(checkout, "src-tauri"), { recursive: true });
    mkdirSync(path.join(checkout, "scripts"), { recursive: true });
    copyFileSync(seeder, path.join(checkout, "scripts/seed-worktree-target.sh"));
  }

  symlinkSync(process.execPath, path.join(stubs, "node"));
  executable(path.join(stubs, "cargo"), cargoStub);
  executable(path.join(stubs, "uname"), '#!/bin/sh\nprintf "%s\\n" "$TEST_OS"\n');
  for (const [name, source] of Object.entries(extraStubs)) executable(path.join(stubs, name), source);

  const worktreeTarget = path.join(worktree, "src-tauri/target");
  return {
    root, main, worktree, mainTarget, worktreeTarget,
    cacheLog: path.join(worktreeTarget, "askhuman-cache.log"),
    cargoCalls: () => (existsSync(cargoLog) ? readFileSync(cargoLog, "utf8").trim().split("\n") : []),
    run(args = [], { checkout = worktree, env = {} } = {}) {
      return spawnSync("bash", [path.join(checkout, "scripts/seed-worktree-target.sh"), ...args], {
        cwd: checkout,
        encoding: "utf8",
        timeout: 30_000,
        env: {
          ...process.env,
          PATH: [stubs, process.env.PATH].join(path.delimiter),
          TEST_OS: os,
          TEST_CARGO_LOG: cargoLog,
          ...env,
        },
      });
    },
  };
}

function assertSuccess(result) {
  assert.equal(result.error, undefined);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
}

const CLEAN = "clean --manifest-path src-tauri/Cargo.toml";
const METADATA = "metadata --format-version 1 --manifest-path src-tauri/Cargo.toml";

test("clones the main checkout's profiles and cleans every path package", darwinOnly, t => {
  const f = fixture(t);
  const result = f.run();
  assertSuccess(result);

  for (const dir of ["local-install", "debug"]) {
    const clone = path.join(f.worktreeTarget, dir, "deps/dep.rlib");
    assert.equal(readFileSync(clone, "utf8"), readFileSync(path.join(f.mainTarget, dir, "deps/dep.rlib"), "utf8"));
    assert.notEqual(statSync(clone).ino, statSync(path.join(f.mainTarget, dir, "deps/dep.rlib")).ino,
      "a clone is a separate file, not a hard link");
  }
  // Only the two profiles used for local work are seeded; module caches never travel.
  assert.equal(existsSync(path.join(f.worktreeTarget, "release")), false);
  assert.equal(existsSync(path.join(f.worktreeTarget, "swift-module-cache")), false);
  assert.equal(existsSync(path.join(f.worktreeTarget,
    "local-install/build/humaninloop-abc123/out/swift-module-cache")), false);
  // The source is left alone.
  assert.equal(readFileSync(path.join(f.mainTarget, "swift-module-cache/shared.pcm"), "utf8"), "shared module cache");
  assert.equal(existsSync(path.join(f.mainTarget,
    "local-install/build/humaninloop-abc123/out/swift-module-cache/legacy.pcm")), true);

  assert.deepEqual(f.cargoCalls(), [
    METADATA,
    `${CLEAN} --profile local-install -p humaninloop -p drag`,
    `${CLEAN} --profile dev -p humaninloop -p drag`,
  ]);
  const log = readFileSync(f.cacheLog, "utf8").trim().split("\n");
  assert.equal(log.length, 1);
  assert.match(log[0], new RegExp(`seed: cloned local-install debug from ${f.main}$`));
  assert.match(result.stdout, /完成（local-install debug）/);
});

test("--dry-run reports the plan and changes nothing", darwinOnly, t => {
  const f = fixture(t);
  const result = f.run(["--dry-run"]);
  assertSuccess(result);
  assert.match(result.stdout, /--dry-run/);
  assert.match(result.stdout, /local-install: 将 clone/);
  assert.match(result.stdout, /debug: 将 clone/);
  assert.equal(existsSync(f.worktreeTarget), false);
  assert.deepEqual(f.cargoCalls(), [METADATA]);
});

test("a profile that already has build output is not overwritten", darwinOnly, t => {
  const f = fixture(t);
  put(path.join(f.worktreeTarget, "local-install/keep.txt"), "my own build output");
  const result = f.run();
  assertSuccess(result);
  assert.match(result.stdout, /local-install: 这里已有构建产物，不覆盖/);
  assert.equal(readFileSync(path.join(f.worktreeTarget, "local-install/keep.txt"), "utf8"), "my own build output");
  assert.equal(existsSync(path.join(f.worktreeTarget, "local-install/deps")), false);
  assert.equal(existsSync(path.join(f.worktreeTarget, "debug/deps/dep.rlib")), true);
  assert.deepEqual(f.cargoCalls(), [METADATA, `${CLEAN} --profile dev -p humaninloop -p drag`]);
  assert.match(readFileSync(f.cacheLog, "utf8"), /seed: cloned debug from /);
});

test("--from picks an explicit source and skips profiles it has no output for", darwinOnly, t => {
  const f = fixture(t);
  const other = path.join(f.root, "other");
  put(path.join(other, "src-tauri/target/debug/deps/other.rlib"), "from the other checkout");
  const result = f.run(["--from", other]);
  assertSuccess(result);
  assert.match(result.stdout, /local-install: 来源没有构建产物，跳过/);
  assert.equal(readFileSync(path.join(f.worktreeTarget, "debug/deps/other.rlib"), "utf8"), "from the other checkout");
  assert.equal(existsSync(path.join(f.worktreeTarget, "debug/deps/dep.rlib")), false);
  assert.equal(existsSync(path.join(f.worktreeTarget, "local-install")), false);
});

test("the source cannot be the checkout the script runs in", darwinOnly, t => {
  const f = fixture(t);
  const result = f.run([], { checkout: f.main });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /来源就是当前检出本身/);
  assert.deepEqual(f.cargoCalls(), []);
});

test("outside macOS it skips with a message and touches nothing", unixOnly, t => {
  const f = fixture(t, { os: "Linux" });
  const result = f.run();
  assertSuccess(result);
  assert.match(result.stdout, /跳过: APFS clone 仅适用于 macOS/);
  assert.equal(existsSync(f.worktreeTarget), false);
  assert.deepEqual(f.cargoCalls(), []);
});

test("a failing cargo clean rolls the cloned profile back", darwinOnly, t => {
  const f = fixture(t);
  const result = f.run([], { env: { TEST_CARGO_CLEAN_STATUS: "1" } });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /local-install 的 cargo clean 失败，已回滚/);
  assert.match(result.stderr, /debug 的 cargo clean 失败，已回滚/);
  assert.equal(existsSync(path.join(f.worktreeTarget, "local-install")), false);
  assert.equal(existsSync(path.join(f.worktreeTarget, "debug")), false);
  assert.equal(existsSync(f.cacheLog), false, "nothing was seeded, so nothing is logged");
});

test("it stops before touching anything when the path packages cannot be listed", darwinOnly, t => {
  const f = fixture(t);
  const result = f.run([], { env: { TEST_CARGO_METADATA: "none" } });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /读不到 cargo metadata/);
  assert.equal(existsSync(f.worktreeTarget), false);
  assert.deepEqual(f.cargoCalls(), [METADATA]);
});

test("it refuses to clone when the volume is not APFS (cp -c would degrade to a full copy)", darwinOnly, t => {
  const f = fixture(t, { stubs: { mount: "#!/bin/sh\necho '/dev/disk9s9 on / (hfs, local, journaled)'\n" } });
  const result = f.run();
  assert.equal(result.status, 1);
  assert.match(result.stderr, /不在同一个 APFS 卷上/);
  assert.equal(existsSync(f.worktreeTarget), false);
  assert.deepEqual(f.cargoCalls(), []);
});

test("an unknown argument is reported even in a non-UTF-8 locale", unixOnly, t => {
  const f = fixture(t);
  const result = f.run(["--bogus"], { env: { LC_ALL: "C", LANG: "C" } });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /未知参数 --bogus（可用/);
});
