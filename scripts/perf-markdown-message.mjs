#!/usr/bin/env node
// Manual Markdown rendering performance probe.
//
// This script opens AskHuman with scripts/perf-markdown-sample.md as the shared
// Markdown message. It is intentionally separate from scripts/perf-popup.mjs:
// run it when you want to look at a real popup with a large Markdown body.
//
// Usage:
//   node scripts/perf-markdown-message.mjs
//   node scripts/perf-markdown-message.mjs --sample docs/overview.md
//   node scripts/perf-markdown-message.mjs --bin ./src-tauri/target/release/AskHuman
//   node scripts/perf-markdown-message.mjs --keep-home

import { spawn, spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(SCRIPT_DIR, "..");
const DEFAULT_SAMPLE = join(SCRIPT_DIR, "perf-markdown-sample.md");
const RUN_TIMEOUT_MS = 5 * 60 * 1000;

function printHelp() {
  const text = readFileSync(new URL(import.meta.url), "utf8");
  for (const line of text.split("\n")) {
    if (line.startsWith("// ")) console.log(line.slice(3));
    else if (line === "//") console.log("");
    else if (line.startsWith("#!")) continue;
    else break;
  }
}

function parseArgs(argv) {
  const opts = {
    bin: null,
    sample: DEFAULT_SAMPLE,
    keepHome: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    switch (arg) {
      case "--bin":
        opts.bin = argv[++i];
        if (!opts.bin) die("--bin requires a path");
        break;
      case "--sample":
      case "--file":
        opts.sample = argv[++i];
        if (!opts.sample) die(`${arg} requires a path`);
        break;
      case "--keep-home":
        opts.keepHome = true;
        break;
      case "-h":
      case "--help":
        printHelp();
        process.exit(0);
        break;
      default:
        die(`unknown option: ${arg}`);
    }
  }
  opts.sample = resolve(REPO_ROOT, opts.sample);
  if (opts.bin) opts.bin = resolve(REPO_ROOT, opts.bin);
  return opts;
}

function die(message, code = 2) {
  console.error(`error: ${message}`);
  process.exit(code);
}

function resolveBin(explicit) {
  const candidates = [];
  if (explicit) candidates.push(explicit);
  if (process.env.ASKHUMAN_BIN) candidates.push(process.env.ASKHUMAN_BIN);
  candidates.push(join(homedir(), ".local", "bin", "AskHuman"));
  candidates.push(join(REPO_ROOT, "src-tauri", "target", "release", "AskHuman"));
  for (const candidate of candidates) {
    if (candidate && existsSync(candidate)) return candidate;
  }
  const which = spawnSync("which", ["AskHuman"], { encoding: "utf8" });
  if (which.status === 0 && which.stdout.trim()) return which.stdout.trim();
  die("could not locate AskHuman; set $ASKHUMAN_BIN, pass --bin, or run ./scripts/install.sh");
}

function screenLockState() {
  if (process.platform !== "darwin") return "unknown";
  const result = spawnSync("ioreg", ["-n", "Root", "-d1", "-r"], { encoding: "utf8" });
  if (result.status !== 0 || !result.stdout) return "unknown";
  const match = result.stdout.match(/"CGSSessionScreenIsLocked"\s*=\s*(Yes|No)/);
  return match ? (match[1] === "Yes" ? "locked" : "unlocked") : "unknown";
}

function assertScreenUsable() {
  if (screenLockState() === "locked") {
    die("screen is locked; unlock it so the popup can paint, then re-run", 1);
  }
}

function startCaffeinate() {
  if (process.platform !== "darwin") return null;
  try {
    return spawn("caffeinate", ["-dimsu"], { stdio: "ignore" });
  } catch {
    return null;
  }
}

function childEnv(home) {
  return {
    ...process.env,
    HOME: home,
    ASKHUMAN_NO_KEYCHAIN: "1",
    ASKHUMAN_ENV_SOURCE_NAME: "Markdown Perf",
  };
}

function writePopupOnlyConfig(home) {
  const dir = join(home, ".askhuman");
  mkdirSync(dir, { recursive: true });
  const config = {
    general: {
      theme: "system",
      language: "zh",
      popupPrewarm: false,
    },
    channels: {
      popup: { enabled: true },
      telegram: { enabled: false },
      dingding: { enabled: false },
      feishu: { enabled: false },
      slack: { enabled: false },
      autoActivation: false,
    },
  };
  writeFileSync(join(dir, "config.json"), JSON.stringify(config, null, 2));
}

function stopDaemon(bin, home) {
  spawnSync(bin, ["daemon", "stop", "--force"], {
    stdio: "ignore",
    env: childEnv(home),
  });
}

function runAskHuman(bin, home, sampleText) {
  return new Promise((resolveRun) => {
    const spawnTs = Date.now();
    const child = spawn(
      bin,
      [
        "--stdin",
        "-q",
        "Markdown 渲染性能观察完成了吗？",
        "-o",
        "看起来正常",
        "-o",
        "需要优化",
      ],
      {
        stdio: ["pipe", "inherit", "inherit"],
        env: {
          ...childEnv(home),
          ASKHUMAN_PERF: "1",
          ASKHUMAN_PERF_SPAWN_TS: String(spawnTs),
        },
      },
    );
    child.stdin.end(sampleText);
    let done = false;
    const finish = (result) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      resolveRun(result);
    };
    const timer = setTimeout(() => {
      try {
        child.kill("SIGKILL");
      } catch {
        // ignore
      }
      finish({ type: "timeout", code: 124 });
    }, RUN_TIMEOUT_MS);
    child.on("exit", (code, signal) => finish({ type: "exit", code, signal }));
    child.on("error", (error) => finish({ type: "error", code: 1, error }));
  });
}

function parsePerfLog(path) {
  if (!existsSync(path)) return {};
  const groups = {};
  for (const line of readFileSync(path, "utf8").split("\n")) {
    if (!line) continue;
    const [tsStr, perfId, stage] = line.split("\t");
    const ts = Number(tsStr);
    if (!perfId || !stage || !Number.isFinite(ts)) continue;
    const group = (groups[perfId] ||= {});
    if (group[stage] === undefined || ts < group[stage]) group[stage] = ts;
  }
  return groups;
}

function latestCompleteRun(groups) {
  return Object.values(groups)
    .filter((group) => group["cli.start"] !== undefined)
    .sort((a, b) => b["cli.start"] - a["cli.start"])[0];
}

function printMetric(group, label, from, to) {
  if (!group || group[from] === undefined || group[to] === undefined) return;
  const delta = group[to] - group[from];
  console.log(`${label.padEnd(30)} ${delta.toFixed(1).padStart(8)} ms`);
}

function printPerfSummary(home) {
  const group = latestCompleteRun(parsePerfLog(join(home, ".askhuman", "perf.log")));
  console.log("");
  console.log("== Popup paint timings ==");
  if (!group || group["fe.painted"] === undefined) {
    console.log("No complete perf marks captured. Is the installed binary built with current instrumentation?");
    return;
  }
  printMetric(group, "spawn -> painted", "spawn", "fe.painted");
  printMetric(group, "cli.start -> painted", "cli.start", "fe.painted");
  printMetric(group, "show -> painted", "gui.show_recv", "fe.painted");
  printMetric(group, "frontend boot -> painted", "fe.bootstrap", "fe.painted");
  printMetric(group, "mounted -> popup_init done", "fe.mounted", "fe.popup_init_done");
  printMetric(group, "popup_init done -> painted", "fe.popup_init_done", "fe.painted");
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!existsSync(opts.sample)) die(`sample file does not exist: ${opts.sample}`);
  assertScreenUsable();

  const bin = resolveBin(opts.bin);
  const sampleText = readFileSync(opts.sample, "utf8");
  const sampleSize = statSync(opts.sample).size;
  const home = mkdtempSync(join(tmpdir(), "askhuman-md-perf-"));
  const caffeinate = startCaffeinate();
  writePopupOnlyConfig(home);

  console.log(`AskHuman:      ${bin}`);
  console.log(`sample:        ${opts.sample} (${sampleSize} bytes)`);
  console.log(`isolated HOME: ${home}`);
  console.log("Open popup:    answer in the AskHuman window when you are done observing.");
  console.log("");

  let exitCode = 0;
  try {
    stopDaemon(bin, home);
    const result = await runAskHuman(bin, home, sampleText);
    if (result.type === "error") {
      console.error(`failed to spawn AskHuman: ${result.error?.message || result.error}`);
      exitCode = 1;
    } else if (result.type === "timeout") {
      console.error("AskHuman timed out after 5 minutes");
      exitCode = 124;
    } else {
      exitCode = result.code ?? (result.signal ? 1 : 0);
      if (result.signal) console.error(`AskHuman exited by signal ${result.signal}`);
    }
    printPerfSummary(home);
  } finally {
    stopDaemon(bin, home);
    if (caffeinate) {
      try {
        caffeinate.kill();
      } catch {
        // ignore
      }
    }
    if (!opts.keepHome) {
      try {
        rmSync(home, { recursive: true, force: true });
      } catch {
        // ignore
      }
    } else {
      console.log(`kept isolated HOME: ${home}`);
    }
  }
  process.exit(exitCode);
}

main().catch((error) => {
  console.error(`error: ${error?.message || error}`);
  process.exit(1);
});
