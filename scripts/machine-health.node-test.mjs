import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  THRESHOLDS,
  classify,
  countProcesses,
  formatReport,
  parseMemoryPressureFreePercent,
  parseMeminfoLinux,
  parseSize,
  parseSwapUsageMac,
  parseTopHeaderMac,
  parseTopRowsMac,
} from "./machine-health.mjs";

const script = fileURLToPath(new URL("./machine-health.mjs", import.meta.url));
const GIB = 1024 ** 3;

// Numbers copied from a machine that was thrashing: 16 GB of RAM, swap exhausted, load ~700.
const THRASHING = {
  platform: "darwin",
  cpuCount: 8,
  load1: 722,
  memTotalBytes: 16 * GIB,
  memFreePercent: 29,
  swapUsedBytes: 21586 * 1024 ** 2,
  swapTotalBytes: 22528 * 1024 ** 2,
  cpu: { user: 40, sys: 60, idle: 0 },
  wiredBytes: 6122 * 1024 ** 2,
  compressorBytes: 5063 * 1024 ** 2,
  topHolders: [{ name: "Cursor Helper (Plugin)", bytes: 1610 * 1024 ** 2 }],
  jobs: { rustc: 3, xcodebuild: 1 },
  bootedSimulators: 5,
};

const HEALTHY = {
  platform: "linux",
  cpuCount: 8,
  load1: 2.5,
  memTotalBytes: 16 * GIB,
  memFreePercent: 60,
  swapUsedBytes: 0,
  swapTotalBytes: 2 * GIB,
  cpu: null,
  topHolders: [],
  jobs: {},
  bootedSimulators: null,
};

test("sizes carry their unit", () => {
  assert.equal(parseSize("22528.00M"), 22528 * 1024 ** 2);
  assert.equal(parseSize("15G"), 15 * GIB);
  assert.equal(parseSize("360M+"), 360 * 1024 ** 2);
  assert.equal(parseSize("512"), 512);
  assert.equal(parseSize("n/a"), null);
});

test("macOS swap usage is parsed from sysctl", () => {
  const swap = parseSwapUsageMac("total = 22528.00M  used = 21586.44M  free = 941.56M  (encrypted)\n");
  assert.equal(swap.totalBytes, 22528 * 1024 ** 2);
  assert.equal(swap.usedBytes, Math.round(21586.44 * 1024 ** 2));
  assert.equal(parseSwapUsageMac("nothing useful"), null);
});

test("the free percentage comes out of memory_pressure's last line", () => {
  assert.equal(parseMemoryPressureFreePercent("Pageouts: 1208923\n\nSystem-wide memory free percentage: 29%\n"), 29);
  assert.equal(parseMemoryPressureFreePercent(""), null);
});

test("the top header gives the CPU split and the memory summary, with or without a compressor", () => {
  const modern = parseTopHeaderMac([
    "Processes: 1766 total, 350 running, 22 stuck, 1394 sleeping, 7027 threads",
    "CPU usage: 40.10% user, 59.89% sys, 0.0% idle",
    "PhysMem: 15G used (6122M wired, 5063M compressor), 360M unused.",
  ].join("\n"));
  assert.deepEqual(modern.cpu, { user: 40.1, sys: 59.89, idle: 0 });
  assert.equal(modern.wiredBytes, 6122 * 1024 ** 2);
  assert.equal(modern.compressorBytes, 5063 * 1024 ** 2);

  const older = parseTopHeaderMac("CPU usage: 3.0% user, 2.0% sys, 95.0% idle\nPhysMem: 15G used (2G wired), 1G unused.");
  assert.equal(older.wiredBytes, 2 * GIB);
  assert.equal(older.compressorBytes, null);
  assert.deepEqual(parseTopHeaderMac("").cpu, null);
});

test("top rows keep command names that contain spaces and drop the change markers", () => {
  const rows = parseTopRowsMac([
    "Processes: 1766 total",
    "",
    "PID    COMMAND          MEM   CMPRS",
    "83501  Cursor Helper (P 1825M 1610M",
    "49067  node             1035M 964M-",
    "1394   Xcode            895M+ 865M",
    "not a row",
  ].join("\n"), 2);
  assert.deepEqual(rows, [
    { pid: 83501, name: "Cursor Helper (P", memBytes: 1825 * 1024 ** 2, compressedBytes: 1610 * 1024 ** 2 },
    { pid: 49067, name: "node", memBytes: 1035 * 1024 ** 2, compressedBytes: 964 * 1024 ** 2 },
  ]);
});

test("Linux memory numbers come from /proc/meminfo", () => {
  const info = parseMeminfoLinux([
    "MemTotal:       16000000 kB",
    "MemAvailable:    4000000 kB",
    "SwapTotal:       2000000 kB",
    "SwapFree:         500000 kB",
  ].join("\n"));
  assert.equal(info.memTotalBytes, 16000000 * 1024);
  assert.equal(info.memFreePercent, 25);
  assert.equal(info.swapTotalBytes, 2000000 * 1024);
  assert.equal(info.swapUsedBytes, 1500000 * 1024);
});

test("only build-related process names are counted, by executable name", () => {
  const counts = countProcesses([
    "/Users/me/.rustup/toolchains/stable/bin/rustc",
    "/Users/me/.cargo/bin/rustc",
    "cargo",
    "/usr/bin/zsh",
    "/Applications/Xcode.app/Contents/Developer/usr/bin/xcodebuild",
  ], ["rustc", "cargo", "xcodebuild", "swift-frontend"]);
  assert.deepEqual(counts, { rustc: 2, cargo: 1, xcodebuild: 1 });
});

test("a healthy machine is ok and says nothing is wrong", () => {
  assert.deepEqual(classify(HEALTHY), { level: "ok", reasons: [] });
});

test("the thrashing machine the numbers came from is classified as thrashing, with the reasons", () => {
  const verdict = classify(THRASHING);
  assert.equal(verdict.level, "thrashing");
  assert.equal(verdict.reasons.length, 3, verdict.reasons.join(" | "));
  assert.match(verdict.reasons.join(" "), /交换区/);
  assert.match(verdict.reasons.join(" "), /负载/);
  assert.match(verdict.reasons.join(" "), /内核态/);
});

test("each signal alone reaches its levels at the documented thresholds", () => {
  const level = (changes) => classify({ ...HEALTHY, ...changes }).level;

  assert.equal(level({ swapUsedBytes: 0.49 * 16 * GIB }), "ok");
  assert.equal(level({ swapUsedBytes: THRESHOLDS.swapToRam.tight * 16 * GIB }), "tight");
  assert.equal(level({ swapUsedBytes: THRESHOLDS.swapToRam.thrashing * 16 * GIB }), "thrashing");

  assert.equal(level({ load1: 2.9 * 8 }), "ok");
  assert.equal(level({ load1: THRESHOLDS.loadPerCore.tight * 8 }), "tight");
  assert.equal(level({ load1: THRESHOLDS.loadPerCore.thrashing * 8 }), "thrashing");

  assert.equal(level({ memFreePercent: 15 }), "ok");
  assert.equal(level({ memFreePercent: 14 }), "tight");
  assert.equal(level({ memFreePercent: 6 }), "thrashing");
});

test("kernel time only counts while the machine has almost no idle time left", () => {
  const level = (cpu) => classify({ ...HEALTHY, cpu }).level;
  // A busy but healthy build: lots of user time, a little kernel time.
  assert.equal(level({ user: 80, sys: 15, idle: 5 }), "ok");
  // Kernel-heavy but with plenty of idle time (for example a large copy): not paging.
  assert.equal(level({ user: 10, sys: 40, idle: 50 }), "ok");
  assert.equal(level({ user: 55, sys: 35, idle: 10 }), "tight");
  assert.equal(level({ user: 40, sys: 60, idle: 0 }), "thrashing");
});

test("signals that could not be read are ignored instead of failing the check", () => {
  assert.equal(
    classify({ platform: "linux", cpuCount: 8, load1: 1, memTotalBytes: 16 * GIB, topHolders: [], jobs: {} }).level,
    "ok",
  );
});

test("the report starts with a greppable verdict line and ends with advice for that level", () => {
  const thrashing = formatReport(THRASHING, classify(THRASHING));
  const first = thrashing.split("\n")[0];
  assert.match(first, /^machine-health: thrashing — /);
  assert.match(thrashing, /已启动的 iOS 模拟器：5 台/);
  assert.match(thrashing, /rustc ×3、xcodebuild ×1/);
  assert.match(thrashing, /Cursor Helper \(Plugin\) 1\.6 GiB/);
  assert.match(thrashing.split("\n").at(-1), /AskHuman/);

  const healthy = formatReport(HEALTHY, classify(HEALTHY));
  assert.equal(healthy.split("\n")[0], "machine-health: ok");
  assert.match(healthy, /正在跑的编译进程：无/);
});

test("running the script on this machine prints a verdict and exits 0, 1 or 2", () => {
  const result = spawnSync(process.execPath, [script], { encoding: "utf8", timeout: 60_000 });
  assert.ok([0, 1, 2].includes(result.status), `status ${result.status}\n${result.stderr}`);
  assert.match(result.stdout, /^machine-health: (ok|tight|thrashing)/);
  assert.equal(result.status, { ok: 0, tight: 1, thrashing: 2 }[/^machine-health: (\w+)/.exec(result.stdout)[1]]);
});
