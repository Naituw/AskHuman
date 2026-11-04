#!/usr/bin/env node
// Machine health check to run before a long build or test.
//
// On a healthy machine a cold build of this repository takes about 3 minutes and a seeded one
// about 80 seconds (docs/agent-worktree-setup.md). When the machine has run out of memory the same
// job takes 10-25 minutes while burning the same CPU time: every process waits for paging. This
// script reads memory, swap, load and CPU numbers, lists the heaviest memory holders and the
// running build jobs, and classifies the machine so an agent can tell the human *before* it starts
// waiting, instead of waiting silently.
//
// Read-only, a few seconds. Exit code: 0 ok, 1 tight, 2 thrashing.
//
//   node scripts/machine-health.mjs

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const GIB = 1024 ** 3;
const UNITS = { K: 1024, M: 1024 ** 2, G: 1024 ** 3, T: 1024 ** 4 };

// A level is reached as soon as any one signal reaches it. "thrashing" is the level at which a
// build is expected to take several times its documented time, so it is the one that justifies
// interrupting the human.
export const THRESHOLDS = {
  // swap in use, as a multiple of physical memory
  swapToRam: { tight: 0.5, thrashing: 1.0 },
  // 1-minute load average per core
  loadPerCore: { tight: 3, thrashing: 10 },
  // share of CPU time spent in the kernel, only while the machine is also nearly out of idle time
  sysPercent: { tight: 30, thrashing: 50 },
  // share of memory still available (lower is worse)
  freePercent: { tight: 15, thrashing: 7 },
};

// Processes that mean "a build or test is running somewhere on this machine".
const JOB_NAMES = ["rustc", "cargo", "xcodebuild", "swift-frontend", "clang"];

export function parseSize(text) {
  const match = /^\s*(\d+(?:\.\d+)?)\s*([KMGT]?)/i.exec(String(text ?? ""));
  if (!match) return null;
  return Math.round(Number(match[1]) * (UNITS[match[2].toUpperCase()] ?? 1));
}

// `sysctl -n vm.swapusage`: "total = 22528.00M  used = 21586.44M  free = 941.56M  (encrypted)"
export function parseSwapUsageMac(text) {
  const match = /total = (\S+)\s+used = (\S+)\s+free = (\S+)/.exec(String(text ?? ""));
  if (!match) return null;
  return { totalBytes: parseSize(match[1]), usedBytes: parseSize(match[2]) };
}

// `memory_pressure`: "System-wide memory free percentage: 29%"
export function parseMemoryPressureFreePercent(text) {
  const match = /System-wide memory free percentage:\s*(\d+)%/.exec(String(text ?? ""));
  return match ? Number(match[1]) : null;
}

// Header of `top -l 1`: the CPU split and the memory summary lines.
export function parseTopHeaderMac(text) {
  const source = String(text ?? "");
  const cpu = /CPU usage:\s*([\d.]+)% user,\s*([\d.]+)% sys,\s*([\d.]+)% idle/.exec(source);
  const memory = /PhysMem:\s*\S+ used \((\S+) wired(?:, (\S+) compressor)?\)/.exec(source);
  return {
    cpu: cpu ? { user: Number(cpu[1]), sys: Number(cpu[2]), idle: Number(cpu[3]) } : null,
    wiredBytes: memory ? parseSize(memory[1]) : null,
    compressorBytes: memory && memory[2] ? parseSize(memory[2]) : null,
  };
}

// Rows of `top -l 1 -stats pid,command,mem,cmprs`; the command column is truncated and may
// contain spaces, so only the last two columns are taken from the right.
export function parseTopRowsMac(text, limit = 6) {
  const rows = [];
  let inTable = false;
  for (const line of String(text ?? "").split("\n")) {
    if (/^PID\s+COMMAND/.test(line)) {
      inTable = true;
      continue;
    }
    if (!inTable) continue;
    const match = /^\s*(\d+)\s+(.+?)\s+(\d+(?:\.\d+)?[KMGT]?)[+-]?\s+(\d+(?:\.\d+)?[KMGT]?)[+-]?\s*$/.exec(line);
    if (match) {
      rows.push({
        pid: Number(match[1]),
        name: match[2].trim(),
        memBytes: parseSize(match[3]),
        compressedBytes: parseSize(match[4]),
      });
    }
  }
  return rows.slice(0, limit);
}

// /proc/meminfo, values in kB.
export function parseMeminfoLinux(text) {
  const field = (name) => {
    const match = new RegExp(`^${name}:\\s+(\\d+) kB`, "m").exec(String(text ?? ""));
    return match ? Number(match[1]) * 1024 : null;
  };
  const total = field("MemTotal");
  const available = field("MemAvailable");
  const swapTotal = field("SwapTotal");
  const swapFree = field("SwapFree");
  return {
    memTotalBytes: total,
    memFreePercent: total && available != null ? Math.round((available / total) * 100) : null,
    swapTotalBytes: swapTotal,
    swapUsedBytes: swapTotal != null && swapFree != null ? swapTotal - swapFree : null,
  };
}

// `ps -eo comm=` lines to a name -> count map restricted to `names`.
export function countProcesses(lines, names) {
  const counts = {};
  for (const line of lines) {
    const name = path.basename(line.trim());
    if (names.includes(name)) counts[name] = (counts[name] ?? 0) + 1;
  }
  return counts;
}

// Pure classification of collected numbers; unknown signals (null) are ignored.
export function classify(metrics) {
  const reasons = [];
  let level = 0;
  const raise = (to, reason) => {
    level = Math.max(level, to);
    reasons.push({ level: to, text: reason });
  };

  if (metrics.swapUsedBytes != null && metrics.memTotalBytes) {
    const ratio = metrics.swapUsedBytes / metrics.memTotalBytes;
    const text = `交换区已用 ${gib(metrics.swapUsedBytes)}，是物理内存的 ${ratio.toFixed(1)} 倍`;
    if (ratio >= THRESHOLDS.swapToRam.thrashing) raise(2, text);
    else if (ratio >= THRESHOLDS.swapToRam.tight) raise(1, text);
  }
  if (metrics.load1 != null && metrics.cpuCount) {
    const perCore = metrics.load1 / metrics.cpuCount;
    const text = `负载 ${Math.round(metrics.load1)}（${metrics.cpuCount} 核，每核 ${perCore.toFixed(1)}）`;
    if (perCore >= THRESHOLDS.loadPerCore.thrashing) raise(2, text);
    else if (perCore >= THRESHOLDS.loadPerCore.tight) raise(1, text);
  }
  if (metrics.cpu) {
    const text = `内核态占 CPU ${Math.round(metrics.cpu.sys)}%，空闲 ${Math.round(metrics.cpu.idle)}%`;
    if (metrics.cpu.sys >= THRESHOLDS.sysPercent.thrashing && metrics.cpu.idle < 10) raise(2, text);
    else if (metrics.cpu.sys >= THRESHOLDS.sysPercent.tight && metrics.cpu.idle < 15) raise(1, text);
  }
  if (metrics.memFreePercent != null) {
    const text = `可用内存仅 ${metrics.memFreePercent}%`;
    if (metrics.memFreePercent < THRESHOLDS.freePercent.thrashing) raise(2, text);
    else if (metrics.memFreePercent < THRESHOLDS.freePercent.tight) raise(1, text);
  }

  return { level: ["ok", "tight", "thrashing"][level], reasons: reasons.map((reason) => reason.text) };
}

function gib(bytes) {
  return `${(bytes / GIB).toFixed(1)} GiB`;
}

function run(command, args, timeoutMs = 8000) {
  try {
    return execFileSync(command, args, {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
      timeout: timeoutMs,
    });
  } catch {
    return null;
  }
}

function processNames(args) {
  const output = run("ps", args);
  return output ? output.split("\n").filter((line) => line.trim()) : [];
}

function collectDarwin(metrics) {
  const swap = parseSwapUsageMac(run("sysctl", ["-n", "vm.swapusage"]));
  if (swap) {
    metrics.swapUsedBytes = swap.usedBytes;
    metrics.swapTotalBytes = swap.totalBytes;
  }
  metrics.memFreePercent = parseMemoryPressureFreePercent(run("memory_pressure"));

  const top = run("top", ["-l", "1", "-o", "cmprs", "-n", "6", "-stats", "pid,command,mem,cmprs"], 15000);
  const header = parseTopHeaderMac(top);
  metrics.cpu = header.cpu;
  metrics.wiredBytes = header.wiredBytes;
  metrics.compressorBytes = header.compressorBytes;
  const rows = parseTopRowsMac(top);
  // top truncates names to 16 characters; ps has the full executable path.
  const full = rows.length ? run("ps", ["-o", "pid=,comm=", "-p", rows.map((row) => row.pid).join(",")]) : null;
  const names = new Map();
  for (const line of (full ?? "").split("\n")) {
    const match = /^\s*(\d+)\s+(.+)$/.exec(line);
    if (match) names.set(Number(match[1]), path.basename(match[2].trim()));
  }
  metrics.topHolders = rows.map((row) => ({
    name: names.get(row.pid) ?? row.name,
    bytes: row.compressedBytes ?? row.memBytes,
  }));

  const commands = processNames(["-axo", "comm="]);
  metrics.jobs = countProcesses(commands, JOB_NAMES);
  // One launchd_sim runs per booted simulator.
  metrics.bootedSimulators = countProcesses(commands, ["launchd_sim"]).launchd_sim ?? 0;
}

function collectLinux(metrics) {
  try {
    Object.assign(metrics, parseMeminfoLinux(readFileSync("/proc/meminfo", "utf8")));
  } catch {
    // keep the os.* numbers
  }
  const output = run("ps", ["-eo", "rss=,comm=", "--sort=-rss"]);
  metrics.topHolders = (output ?? "")
    .split("\n")
    .map((line) => /^\s*(\d+)\s+(.+)$/.exec(line))
    .filter(Boolean)
    .slice(0, 6)
    .map((match) => ({ name: path.basename(match[2].trim()), bytes: Number(match[1]) * 1024 }));
  metrics.jobs = countProcesses(processNames(["-eo", "comm="]), JOB_NAMES);
}

export function collect(platform = process.platform) {
  const metrics = {
    platform,
    cpuCount: os.availableParallelism?.() ?? os.cpus().length,
    load1: os.loadavg()[0],
    memTotalBytes: os.totalmem(),
    memFreePercent: null,
    swapUsedBytes: null,
    swapTotalBytes: null,
    cpu: null,
    wiredBytes: null,
    compressorBytes: null,
    topHolders: [],
    jobs: {},
    bootedSimulators: null,
  };
  if (platform === "darwin") collectDarwin(metrics);
  else if (platform === "linux") collectLinux(metrics);
  return metrics;
}

const ADVICE = {
  ok: "机器状态正常，可以直接编译。",
  tight:
    "内存或负载偏紧：编译可能比文档基线（冷编译约 3 分钟，播种后首次约 1 分 20 秒）慢几倍。"
    + "长任务放后台，分段轮询并说明在等什么。",
  thrashing:
    "机器正在换页：文档基线 1–3 分钟的编译可能拖到 10–25 分钟，CPU 用量不变，全花在等换页上。"
    + "先用 AskHuman 把这份报告告诉人类并请他释放内存（上面占用最多的进程、已启动的模拟器、Xcode、Cursor 多余窗口），"
    + "任务可以先放后台跑；不要静默干等。",
};

export function formatReport(metrics, verdict) {
  const lines = [];
  const head = verdict.reasons.length ? ` — ${verdict.reasons.join("；")}` : "";
  lines.push(`machine-health: ${verdict.level}${head}`);

  const memory = [`内存 ${gib(metrics.memTotalBytes)}`];
  if (metrics.memFreePercent != null) memory.push(`可用 ${metrics.memFreePercent}%`);
  if (metrics.compressorBytes != null) memory.push(`压缩器 ${gib(metrics.compressorBytes)}`);
  if (metrics.wiredBytes != null) memory.push(`wired ${gib(metrics.wiredBytes)}`);
  lines.push(`  ${memory.join("，")}`);
  if (metrics.swapUsedBytes != null) {
    lines.push(`  交换区 已用 ${gib(metrics.swapUsedBytes)} / 共 ${gib(metrics.swapTotalBytes ?? 0)}`);
  }
  lines.push(`  负载 ${Math.round(metrics.load1)}（${metrics.cpuCount} 核）`);
  if (metrics.cpu) {
    lines.push(`  CPU ${Math.round(metrics.cpu.user)}% user / ${Math.round(metrics.cpu.sys)}% sys / ${Math.round(metrics.cpu.idle)}% idle`);
  }
  const jobs = Object.entries(metrics.jobs ?? {}).map(([name, count]) => `${name} ×${count}`);
  lines.push(`  正在跑的编译进程：${jobs.length ? jobs.join("、") : "无"}`);
  if (metrics.bootedSimulators != null) lines.push(`  已启动的 iOS 模拟器：${metrics.bootedSimulators} 台`);
  if (metrics.topHolders?.length) {
    const label = metrics.platform === "darwin" ? "占内存最多（压缩后）" : "占内存最多";
    lines.push(`  ${label}：${metrics.topHolders.map((holder) => `${holder.name} ${gib(holder.bytes)}`).join("、")}`);
  }
  lines.push(`  建议：${ADVICE[verdict.level]}`);
  return lines.join("\n");
}

function main() {
  if (process.platform === "win32") {
    console.log("machine-health: ok — Windows 暂不检查，已跳过。");
    return;
  }
  const metrics = collect();
  const verdict = classify(metrics);
  console.log(formatReport(metrics, verdict));
  process.exitCode = { ok: 0, tight: 1, thrashing: 2 }[verdict.level];
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
