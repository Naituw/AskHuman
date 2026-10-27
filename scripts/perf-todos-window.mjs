#!/usr/bin/env node
// Todos-window open performance harness (spec docs/specs/todo-sync.md §9.9, plan P0 / P1).
//
// `node scripts/perf-todos-window.mjs` runs a FIXED scenario against a FIXED baseline
// (docs/perf/todos-window-baseline.json):
//   * No baseline yet   -> measures and writes it.
//   * Baseline exists   -> measures, prints the tables, exits non-zero on a regression.
//   * --update-baseline -> overwrite the baseline with the current numbers.
//
// Scenario, per data tier:
//   * Throwaway $HOME + ASKHUMAN_HOME (its own GUI host, socket, perf.log; ASKHUMAN_NO_KEYCHAIN keeps
//     it off the real keychain) holding synthetic todos.json / agent-workspaces.json:
//       real   = the SHAPE of a real data set (8 projects, 19 todos, 72 history entries, 12 workspaces),
//       stress = 200 projects, 2000 todos, 2000 history entries, 50 workspaces.
//     The window is opened by `AskHuman --todos` from inside the busiest project, so the first paint
//     renders a real queue + history.
//   * cold set = GUI host started fresh before every run (first WebView of the process),
//     hot set  = one long-lived host; each run opens the window again after the previous one closed.
//   * The window closes itself right after its first paint (ASKHUMAN_PERF_AUTODISMISS), so no click
//     is needed. Milestones come from `todos.*` / `fe.*` lines in perf.log (see src-tauri/src/perf.rs).
//
// Prerequisite: ./scripts/install.sh (or ASKHUMAN_BIN=<path>) so the binary carries the marks.
// The window must paint on a visible display: the harness refuses to run while the screen is locked
// and keeps the display awake with `caffeinate`.
//
// A/B mode (`--ref <binary>`): when the machine is shared and busy, absolute numbers are not
// comparable between runs, so the candidate is measured against a reference binary instead. Both
// binaries run the same scenario INTERLEAVED (ref, cand, cand, ref, ...), which makes the machine
// load common-mode; the gate is then candidate vs reference. The baseline file is not touched.
//
// Usage:
//   node scripts/perf-todos-window.mjs
//   node scripts/perf-todos-window.mjs --update-baseline
//   node scripts/perf-todos-window.mjs --ref <binary>   # A/B: candidate (ASKHUMAN_BIN or the dev install) vs <binary>
//   node scripts/perf-todos-window.mjs --tier real      # debugging only; never writes the baseline
//   node scripts/perf-todos-window.mjs --keep-home      # keep the temp $HOME on exit

import { spawn, spawnSync } from "node:child_process";
import {
  existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync,
} from "node:fs";
import { cpus, homedir, loadavg, tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// ---- fixed scenario knobs (built in on purpose) ----------------------------
const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE_PATH = join(REPO_ROOT, "docs", "perf", "todos-window-baseline.json");
const COLD_RUNS = 8, COLD_WARMUP = 1;
const HOT_RUNS = 24, HOT_WARMUP = 4;
const RUN_TIMEOUT_MS = 20000;
const HOST_READY_TIMEOUT_MS = 15000;
const SETTLE_MS = 500; // after a first paint: let the window finish closing before the next open
// Regression gate (compared per tier + set, on the p90 of the e2e and frontend segments): the run
// fails only when BOTH the relative and the absolute increase exceed these, so timer noise on the
// small segments cannot trip it.
const GATE_REL_PCT = 10;
const GATE_ABS_MS = 30;
const STRESS_FRONTEND_P90_MAX_MS = 300; // absolute budget for the stress tier's hot set (spec §9.9)

const TIERS = {
  real: {
    todoCounts: [9, 3, 2, 1, 1, 1, 1, 1],
    histCounts: [54, 10, 5, 3],
    workspaces: 12,
  },
  stress: {
    projects: 200,
    totalTodos: 2000,
    histProjects: 40,
    histPerProject: 50,
    workspaces: 50,
  },
};

// ---- args ------------------------------------------------------------------

function parseArgs(argv) {
  const o = { updateBaseline: false, keepHome: false, tier: null, ref: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case "--update-baseline": o.updateBaseline = true; break;
      case "--keep-home": o.keepHome = true; break;
      case "--tier": o.tier = argv[++i]; break;
      case "--ref": o.ref = argv[++i]; break;
      case "-h": case "--help": printHelp(); process.exit(0); break;
      default:
        console.error(`unknown option: ${a}`);
        printHelp();
        process.exit(2);
    }
  }
  if (o.tier && !TIERS[o.tier]) {
    console.error(`unknown tier: ${o.tier} (real | stress)`);
    process.exit(2);
  }
  if (o.tier && o.updateBaseline) {
    console.error("--tier is for debugging; the baseline needs every tier");
    process.exit(2);
  }
  if (o.ref && o.updateBaseline) {
    console.error("--ref compares two binaries; it never writes the baseline");
    process.exit(2);
  }
  if (o.ref && !existsSync(o.ref)) {
    console.error(`--ref binary not found: ${o.ref}`);
    process.exit(2);
  }
  return o;
}

function printHelp() {
  const text = readFileSync(new URL(import.meta.url)).toString();
  for (const l of text.split("\n")) {
    if (l.startsWith("// ")) console.log(l.slice(3));
    else if (l === "//") console.log("");
    else if (l.startsWith("#!")) continue;
    else break;
  }
}

function resolveBin() {
  const candidates = [];
  if (process.env.ASKHUMAN_BIN) candidates.push(process.env.ASKHUMAN_BIN);
  candidates.push(join(REPO_ROOT, ".askhuman-dev", "bin", "AskHuman"));
  candidates.push(join(homedir(), ".local", "bin", "AskHuman"));
  for (const c of candidates) if (c && existsSync(c)) return c;
  console.error("could not locate the AskHuman binary; set $ASKHUMAN_BIN or run scripts/install.sh");
  process.exit(2);
}

// ---- display / lock guard --------------------------------------------------

function screenLockState() {
  if (process.platform !== "darwin") return "unknown";
  const r = spawnSync("ioreg", ["-n", "Root", "-d1", "-r"], { encoding: "utf8" });
  if (r.status !== 0 || !r.stdout) return "unknown";
  const m = r.stdout.match(/"CGSSessionScreenIsLocked"\s*=\s*(Yes|No)/);
  return m ? (m[1] === "Yes" ? "locked" : "unlocked") : "unknown";
}

function assertScreenUsable(where) {
  if (screenLockState() === "locked") {
    throw new Error(
      `screen is locked (${where}); the window can't paint so latency is unmeasurable. ` +
        "Unlock the screen, keep it awake, and re-run.",
    );
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

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Timings taken while other jobs saturate the machine are not comparable, so wait (bounded) for the
 *  1-minute load average to drop below ~60% of the cores; proceed with a warning if it never does. */
const QUIET_LOAD_FRACTION = 0.6;
const QUIET_WAIT_MS = 10 * 60_000;
async function waitQuiet() {
  const limit = cpus().length * QUIET_LOAD_FRACTION;
  const deadline = Date.now() + QUIET_WAIT_MS;
  let announced = false;
  while (loadavg()[0] > limit) {
    if (Date.now() > deadline) {
      console.log(`warning: the machine stayed busy (load ${loadavg()[0].toFixed(1)} > ${limit.toFixed(1)}); numbers are noisy`);
      return false;
    }
    if (!announced) {
      console.log(`waiting for the machine to calm down (load ${loadavg()[0].toFixed(1)} > ${limit.toFixed(1)}) ...`);
      announced = true;
    }
    await sleep(10_000);
  }
  return true;
}

// ---- synthetic data --------------------------------------------------------

function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const WORDS = [
  "fix", "the", "flaky", "test", "in", "sync", "layer", "update", "docs", "for", "settings", "page",
  "refactor", "popup", "channel", "router", "add", "retry", "with", "backoff", "split", "module",
  "benchmark", "startup", "path", "cleanup", "dead", "code", "review", "feedback", "from", "Alice",
  "修复", "待办", "窗口", "打开", "速度", "同步", "设置", "页面", "补充", "测试", "整理", "文档",
];

function sentence(rand) {
  const n = 3 + Math.floor(rand() * 18);
  const parts = [];
  for (let i = 0; i < n; i++) parts.push(WORDS[Math.floor(rand() * WORDS.length)]);
  let s = parts.join(" ");
  if (rand() < 0.1) s += "\n" + parts.slice(0, 4).join(" ");
  return s;
}

function uid(rand) {
  let s = "";
  for (let i = 0; i < 32; i++) s += Math.floor(rand() * 16).toString(16);
  return `${s.slice(0, 8)}-${s.slice(8, 12)}-${s.slice(12, 16)}-${s.slice(16, 20)}-${s.slice(20)}`;
}

/** Build todos.json + the workspace index for a tier; returns the preselect directory. */
function writeTierData(home, tierName) {
  const tier = TIERS[tierName];
  const rand = rng(tierName === "real" ? 7 : 11);
  const work = join(home, "work");
  const mkProject = (name) => {
    const dir = join(work, name);
    mkdirSync(join(dir, ".git"), { recursive: true });
    return realpathSync(dir);
  };

  let todoCounts;
  let histCounts;
  let projectCount;
  if (tierName === "real") {
    todoCounts = tier.todoCounts;
    histCounts = tier.histCounts;
    projectCount = todoCounts.length;
  } else {
    projectCount = tier.projects;
    todoCounts = new Array(projectCount).fill(1);
    // Spread the rest randomly but keep project 0 the busiest (it is the preselected one).
    let left = tier.totalTodos - projectCount - 24;
    todoCounts[0] += 24;
    while (left-- > 0) todoCounts[1 + Math.floor(rand() * (projectCount - 1))]++;
    histCounts = new Array(tier.histProjects).fill(tier.histPerProject);
    histCounts[0] = 100;
    histCounts[1] = 100;
  }

  const keys = [];
  for (let i = 0; i < projectCount; i++) keys.push(mkProject(`proj-${String(i).padStart(3, "0")}`));

  const now = Date.now();
  const projects = {};
  keys.forEach((key, i) => {
    const list = [];
    for (let j = 0; j < todoCounts[i]; j++) {
      const e = {
        id: uid(rand),
        text: sentence(rand),
        createdAtMs: now - (todoCounts[i] - j) * 3600_000,
      };
      if (rand() < 0.2) e.agentKind = rand() < 0.5 ? "cursor" : "claude";
      if (rand() < 0.1) e.auto = true;
      list.push(e);
    }
    projects[key] = list;
  });
  const history = {};
  histCounts.forEach((count, i) => {
    const key = keys[i] ?? mkProject(`hist-${i}`);
    const list = [];
    for (let j = 0; j < count; j++) {
      list.push({
        id: uid(rand),
        text: sentence(rand),
        createdAtMs: now - (count + 5 - j) * 7200_000,
        doneAtMs: now - (count - j) * 3600_000,
        ...(rand() < 0.2 ? { agentKind: "cursor" } : {}),
      });
    }
    history[key] = list;
  });

  const cfgDir = join(home, ".askhuman");
  mkdirSync(join(cfgDir, "state"), { recursive: true });
  writeFileSync(join(cfgDir, "state", "todos.json"), JSON.stringify({ projects, history }));

  const workspaces = [];
  for (let i = 0; i < tier.workspaces; i++) {
    const path = mkProject(`ws-${String(i).padStart(3, "0")}`);
    workspaces.push({
      path,
      label: `ws-${i}`,
      lastUsedAt: now - i * 60_000,
      agents: ["cursor"],
      pinned: i === 0,
      hidden: false,
    });
  }
  writeFileSync(join(cfgDir, "agent-workspaces.json"), JSON.stringify({ workspaces }));
  writeFileSync(
    join(cfgDir, "config.json"),
    JSON.stringify({ general: { theme: "system", language: "en" }, channels: { popup: { enabled: true } } }),
  );
  return keys[0];
}

// ---- processes -------------------------------------------------------------

function childEnv(home, perfId) {
  return {
    ...process.env,
    HOME: home,
    ASKHUMAN_HOME: join(home, ".askhuman"),
    ASKHUMAN_NO_KEYCHAIN: "1",
    ASKHUMAN_PERF: "1",
    ASKHUMAN_PERF_ID: perfId,
    ASKHUMAN_PERF_AUTODISMISS: "1",
  };
}

/** Hosts that are currently running, so an interrupted run can still stop them. */
const liveHosts = new Set();

function startHost(bin, home, perfId) {
  const sock = join(home, ".askhuman", "gui-host.sock");
  rmSync(sock, { force: true });
  const child = spawn(bin, ["--gui-host"], {
    cwd: home,
    stdio: "ignore",
    env: childEnv(home, perfId),
    detached: false,
  });
  const host = { child, sock };
  liveHosts.add(host);
  return host;
}

async function waitHostReady(host) {
  const deadline = Date.now() + HOST_READY_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (host.child.exitCode !== null) throw new Error("GUI host exited before it was ready");
    if (existsSync(host.sock)) {
      await sleep(250); // the socket file appears slightly before the accept loop
      return;
    }
    await sleep(40);
  }
  throw new Error("GUI host did not become ready in time");
}

async function stopHost(host, bin, home) {
  liveHosts.delete(host);
  try { host.child.kill("SIGTERM"); } catch { /* ignore */ }
  const deadline = Date.now() + 5000;
  while (host.child.exitCode === null && Date.now() < deadline) await sleep(50);
  if (host.child.exitCode === null) {
    try { host.child.kill("SIGKILL"); } catch { /* ignore */ }
  }
  spawnSync(bin, ["daemon", "stop", "--force"], { stdio: "ignore", env: childEnv(home, "") });
  rmSync(join(home, ".askhuman", "gui-host.sock"), { force: true });
  rmSync(join(home, ".askhuman", "gui-host.lock"), { force: true });
}

/** Parse perf.log lines for `perfId` into [{ts, stage}] sorted by time. */
function readMarks(path, perfId) {
  if (!existsSync(path)) return [];
  const out = [];
  for (const line of readFileSync(path, "utf8").split("\n")) {
    if (!line) continue;
    const [tsStr, id, stage] = line.split("\t");
    const ts = Number(tsStr);
    if (id === perfId && stage && Number.isFinite(ts)) out.push({ ts, stage });
  }
  return out.sort((a, b) => a.ts - b.ts);
}

/** Open the window once (`AskHuman --todos`) and wait for its first paint. Returns the stage map. */
async function openOnce(bin, home, perfId, cwd, perfLog) {
  const t0 = Date.now();
  const child = spawn(bin, ["--todos"], { cwd, stdio: "ignore", env: childEnv(home, perfId) });
  child.on("error", () => {});
  const deadline = t0 + RUN_TIMEOUT_MS;
  let marks = [];
  while (Date.now() < deadline) {
    marks = readMarks(perfLog, perfId).filter((m) => m.ts >= t0);
    if (marks.some((m) => m.stage === "todos.fe.firstPaint")) break;
    await sleep(20);
  }
  try { child.kill("SIGKILL"); } catch { /* ignore */ }
  const stages = { t0 };
  for (const m of marks) if (stages[m.stage] === undefined) stages[m.stage] = m.ts;
  return stages;
}

// ---- metrics ---------------------------------------------------------------

// [label, fromStage, toStage]; indented labels are sub-segments of the line above them.
const METRICS = [
  ["e2e (open->first paint)", "t0", "todos.fe.firstPaint"],
  ["  cli+host (->host.open)", "t0", "todos.host.open"],
  ["  window create (create->built)", "todos.win.create", "todos.win.built"],
  ["  page boot (built->fe.bootstrap)", "todos.win.built", "fe.bootstrap"],
  ["  vue mount (bootstrap->mounted)", "fe.bootstrap", "fe.mounted"],
  ["  init ipc (mounted->init)", "fe.mounted", "todos.fe.init"],
  ["  listeners+data (init->dataReady)", "todos.fe.init", "todos.fe.dataReady"],
  ["  render (dataReady->first paint)", "todos.fe.dataReady", "todos.fe.firstPaint"],
  ["frontend (bootstrap->first paint)", "fe.bootstrap", "todos.fe.firstPaint"],
  ["background enrich (dataReady->enriched)", "todos.fe.dataReady", "todos.fe.enriched"],
];
const E2E = METRICS[0][0];
const FRONTEND = "frontend (bootstrap->first paint)";

function percentile(sorted, p) {
  if (sorted.length === 0) return null;
  if (sorted.length === 1) return sorted[0];
  const rank = (p / 100) * (sorted.length - 1);
  const lo = Math.floor(rank), hi = Math.ceil(rank);
  return lo === hi ? sorted[lo] : sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo);
}

function summarize(values) {
  const v = values.filter((x) => Number.isFinite(x)).sort((a, b) => a - b);
  if (v.length === 0) return { count: 0, min: null, median: null, p90: null, max: null };
  return { count: v.length, min: v[0], median: percentile(v, 50), p90: percentile(v, 90), max: v[v.length - 1] };
}

function aggregate(runs, warmup) {
  const measured = runs.slice(warmup);
  const complete = measured.filter((g) => g["todos.fe.firstPaint"] !== undefined);
  const metrics = {};
  for (const [label, from, to] of METRICS) {
    const vals = [];
    for (const g of complete) {
      if (g[from] !== undefined && g[to] !== undefined) vals.push(g[to] - g[from]);
    }
    metrics[label] = summarize(vals);
  }
  return { complete: complete.length, total: measured.length, metrics };
}

// ---- running ---------------------------------------------------------------

class NoPaintError extends Error {}

/** One failed attempt is retried once (a WebView that is occluded or throttled by a busy machine
 *  occasionally misses a paint); the retries are counted and reported so flakiness stays visible. */
const MAX_RETRIES_TOTAL = 4;
let retriesTotal = 0;

async function withRetry(label, fn) {
  for (let attempt = 0; ; attempt++) {
    try {
      return await fn(attempt);
    } catch (err) {
      if (!(err instanceof NoPaintError) || attempt >= 1 || retriesTotal >= MAX_RETRIES_TOTAL) throw err;
      retriesTotal++;
      process.stdout.write(`\n  ${label}: ${err.message} -> retrying (retry ${retriesTotal}/${MAX_RETRIES_TOTAL})\n`);
    }
  }
}

function noPaint(label, stages) {
  const seen = Object.keys(stages).filter((k) => k !== "t0").join(", ") || "none";
  return new NoPaintError(
    `${label}: the window never painted (marks seen: ${seen})` +
      (screenLockState() === "locked" ? " (screen is locked)" : ""),
  );
}

/** One binary under test with its own throwaway home (and therefore its own GUI host). */
function makeSubject(label, bin, tierName, homes) {
  // Short path on purpose: the host's unix socket lives under it and macOS limits socket paths to
  // 104 bytes (the per-user $TMPDIR is already ~60).
  const home = realpathSync(mkdtempSync(process.platform === "win32" ? join(tmpdir(), "ahp-") : "/tmp/ahp-"));
  homes.push(home); // registered right away so a failed run still cleans up (or keeps) it
  const cwd = writeTierData(home, tierName);
  return {
    label, bin, home, cwd,
    perfLog: join(home, ".askhuman", "perf.log"),
    coldRuns: [], hotRuns: [], host: null, hotGeneration: 0,
  };
}

/** Runs one tier for every subject, INTERLEAVED when there are several (order alternates per round). */
async function runTier(tierName, bins, homes) {
  const subjects = bins.map(([label, bin]) => makeSubject(label, bin, tierName, homes));
  const multi = subjects.length > 1;
  const order = (i) => (i % 2 === 0 ? subjects : [...subjects].reverse());
  const nameOf = (s, what) => `${tierName} ${multi ? `${s.label} ` : ""}${what}`;

  // Cold set: a fresh GUI host per run (the first WebView of that process).
  const coldTotal = COLD_RUNS + COLD_WARMUP;
  process.stdout.write(`[${tierName}] cold set (${coldTotal} runs${multi ? " x2 binaries, interleaved" : ""}, fresh host each)\n`);
  for (let i = 0; i < coldTotal; i++) {
    for (const s of order(i)) {
      const label = nameOf(s, `cold run ${i + 1}`);
      const stages = await withRetry(label, async (attempt) => {
        assertScreenUsable(label);
        const perfId = `cold-${tierName}-${s.label}-${i}-${attempt}`;
        const host = startHost(s.bin, s.home, perfId);
        try {
          await waitHostReady(host);
          const got = await openOnce(s.bin, s.home, perfId, s.cwd, s.perfLog);
          if (got["todos.fe.firstPaint"] === undefined) throw noPaint(label, got);
          return got;
        } finally {
          await stopHost(host, s.bin, s.home);
        }
      });
      s.coldRuns.push(stages);
    }
    process.stdout.write(`\r  ${i + 1}/${coldTotal}   `);
  }
  process.stdout.write("\n");

  // Hot set: one long-lived host per subject; every run opens a new window after the previous one
  // closed. (A retry restarts that subject's host first: whatever wedged it may live in the host.)
  const hotTotal = HOT_RUNS + HOT_WARMUP + 1;
  process.stdout.write(`[${tierName}] hot set (${HOT_RUNS + HOT_WARMUP} runs${multi ? " x2 binaries, interleaved" : ""}, one host each)\n`);
  const startHot = async (s) => {
    const perfId = `hot-${tierName}-${s.label}-${s.hotGeneration++}`;
    s.host = { ...startHost(s.bin, s.home, perfId), perfId };
    await waitHostReady(s.host);
  };
  try {
    for (const s of subjects) await startHot(s);
    // Run 0 is the first open of each host's life (cold-ish): kept out of the hot set.
    for (let i = 0; i < hotTotal; i++) {
      for (const s of order(i)) {
        const label = nameOf(s, `hot run ${i + 1}`);
        const stages = await withRetry(label, async (attempt) => {
          assertScreenUsable(label);
          if (attempt > 0) {
            await stopHost(s.host, s.bin, s.home);
            await startHot(s);
          }
          const got = await openOnce(s.bin, s.home, s.host.perfId, s.cwd, s.perfLog);
          if (got["todos.fe.firstPaint"] === undefined) throw noPaint(label, got);
          return got;
        });
        if (i > 0) s.hotRuns.push(stages);
        await sleep(SETTLE_MS);
      }
      process.stdout.write(`\r  ${i + 1}/${hotTotal}   `);
    }
  } finally {
    for (const s of subjects) if (s.host) await stopHost(s.host, s.bin, s.home);
  }
  process.stdout.write("\n");

  const out = {};
  for (const s of subjects) {
    out[s.label] = { cold: aggregate(s.coldRuns, COLD_WARMUP), hot: aggregate(s.hotRuns, HOT_WARMUP) };
  }
  return out;
}

// ---- reporting -------------------------------------------------------------

const fmt = (n) => (n === null || n === undefined ? "-" : n.toFixed(1));
const signed = (d) => `${d >= 0 ? "+" : ""}${d.toFixed(1)}`;

function printTable(title, agg, base) {
  console.log("");
  console.log(`== ${title} ==  ${agg.complete} complete / ${agg.total} runs`);
  const head =
    "segment".padEnd(42) + "n".padStart(4) + "min".padStart(8) + "median".padStart(8) +
    "p90".padStart(8) + "max".padStart(8) + (base ? "  base p90".padStart(11) + "  delta" : "");
  console.log(head);
  console.log("-".repeat(head.length));
  for (const [label] of METRICS) {
    const m = agg.metrics[label];
    let row =
      label.padEnd(42) + String(m.count).padStart(4) + fmt(m.min).padStart(8) +
      fmt(m.median).padStart(8) + fmt(m.p90).padStart(8) + fmt(m.max).padStart(8);
    if (base) {
      const b = base.metrics?.[label];
      if (b?.p90 != null && m.p90 != null) {
        row += fmt(b.p90).padStart(11) + `  ${signed(m.p90 - b.p90)}`;
      } else {
        row += "-".padStart(11);
      }
    }
    console.log(row);
  }
}

/** A/B table: reference vs candidate, medians and p90s. */
function printCompare(title, ref, cand) {
  console.log("");
  console.log(`== ${title} ==  ref ${ref.complete}/${ref.total} runs, cand ${cand.complete}/${cand.total} runs`);
  const head =
    "segment".padEnd(42) + "ref med".padStart(9) + "cand med".padStart(10) + "delta".padStart(8) +
    "ref p90".padStart(10) + "cand p90".padStart(10) + "delta".padStart(8);
  console.log(head);
  console.log("-".repeat(head.length));
  for (const [label] of METRICS) {
    const r = ref.metrics[label];
    const c = cand.metrics[label];
    const d = (a, b) => (a != null && b != null ? signed(b - a) : "-");
    console.log(
      label.padEnd(42) + fmt(r.median).padStart(9) + fmt(c.median).padStart(10) +
        d(r.median, c.median).padStart(8) + fmt(r.p90).padStart(10) + fmt(c.p90).padStart(10) +
        d(r.p90, c.p90).padStart(8),
    );
  }
}

/** Returns the list of failure strings for one tier + set (`base` is a baseline entry or the ref run). */
function gate(name, agg, base, tierName, setName, what = "baseline") {
  const failures = [];
  for (const label of [E2E, FRONTEND]) {
    const cur = agg.metrics[label]?.p90;
    const b = base?.metrics?.[label]?.p90;
    if (cur == null || b == null || b <= 0) continue;
    const abs = cur - b;
    const rel = (abs / b) * 100;
    if (rel > GATE_REL_PCT && abs > GATE_ABS_MS) {
      failures.push(
        `${name}: ${label} p90 ${cur.toFixed(1)} ms vs ${what} ${b.toFixed(1)} ms ` +
          `(+${abs.toFixed(1)} ms, +${rel.toFixed(1)}% > ${GATE_REL_PCT}% and > ${GATE_ABS_MS} ms)`,
      );
    }
  }
  if (tierName === "stress" && setName === "hot") {
    const cur = agg.metrics[FRONTEND]?.p90;
    if (cur != null && cur > STRESS_FRONTEND_P90_MAX_MS) {
      failures.push(`${name}: ${FRONTEND} p90 ${cur.toFixed(1)} ms > ${STRESS_FRONTEND_P90_MAX_MS} ms budget`);
    }
  }
  return failures;
}

// ---- cleanup on interruption -------------------------------------------------

const cleanup = { caffeinate: null, homes: [], keepHome: false };

function cleanupNow() {
  for (const host of liveHosts) {
    try { host.child.kill("SIGKILL"); } catch { /* ignore */ }
  }
  liveHosts.clear();
  if (cleanup.caffeinate) try { cleanup.caffeinate.kill(); } catch { /* ignore */ }
  if (!cleanup.keepHome) for (const h of cleanup.homes) rmSync(h, { recursive: true, force: true });
}

for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(sig, () => {
    cleanupNow();
    process.exit(130);
  });
}

async function main() {
  const o = parseArgs(process.argv.slice(2));
  const bin = resolveBin();
  assertScreenUsable("at start");
  const tierNames = o.tier ? [o.tier] : Object.keys(TIERS);
  const baseline = !o.ref && existsSync(BASELINE_PATH) ? JSON.parse(readFileSync(BASELINE_PATH, "utf8")) : null;
  const bins = o.ref ? [["ref", o.ref], ["cand", bin]] : [["cand", bin]];
  const caffeinate = startCaffeinate();
  const results = {}; // tier -> label -> {cold, hot}
  const homes = [];
  cleanup.caffeinate = caffeinate;
  cleanup.homes = homes;
  cleanup.keepHome = o.keepHome;
  let loadavgAtStart = [];
  try {
    console.log(`candidate: ${bin}`);
    if (o.ref) console.log(`reference: ${o.ref}`);
    await waitQuiet();
    loadavgAtStart = loadavg().map((x) => Number(x.toFixed(2)));
    console.log(`load average at start: ${loadavg().map((x) => x.toFixed(2)).join(" ")} (${cpus().length} cores)`);
    for (const tierName of tierNames) {
      results[tierName] = await runTier(tierName, bins, homes);
    }
  } finally {
    if (caffeinate) try { caffeinate.kill(); } catch { /* ignore */ }
    if (!o.keepHome) for (const h of homes) rmSync(h, { recursive: true, force: true });
    else console.log(`kept temp homes: ${homes.join(", ")}`);
  }

  let failures = [];
  for (const [tierName, byLabel] of Object.entries(results)) {
    for (const setName of ["cold", "hot"]) {
      const cand = byLabel.cand[setName];
      if (o.ref) {
        const ref = byLabel.ref[setName];
        printCompare(`${tierName} / ${setName}`, ref, cand);
        failures = failures.concat(gate(`${tierName}/${setName}`, cand, ref, tierName, setName, "reference"));
      } else {
        const base = baseline?.tiers?.[tierName]?.[setName];
        printTable(`${tierName} / ${setName}`, cand, base);
        failures = failures.concat(gate(`${tierName}/${setName}`, cand, base, tierName, setName));
      }
    }
  }

  if (!o.ref && (o.updateBaseline || (!baseline && !o.tier))) {
    const tiers = {};
    for (const [tierName, byLabel] of Object.entries(results)) tiers[tierName] = byLabel.cand;
    mkdirSync(dirname(BASELINE_PATH), { recursive: true });
    writeFileSync(
      BASELINE_PATH,
      JSON.stringify(
        {
          generatedAt: new Date().toISOString(),
          loadAverageAtStart: loadavgAtStart,
          retries: retriesTotal,
          gate: { relPct: GATE_REL_PCT, absMs: GATE_ABS_MS, stressFrontendP90MaxMs: STRESS_FRONTEND_P90_MAX_MS },
          tiers,
        },
        null,
        2,
      ) + "\n",
    );
    console.log(`\nbaseline written: ${BASELINE_PATH}`);
    return;
  }
  if (failures.length) {
    console.error("");
    for (const f of failures) console.error(`REGRESSION ${f}`);
    process.exit(1);
  }
  const what = o.ref ? "the reference binary" : baseline ? "the baseline" : null;
  console.log(what ? `\nOK: within the gate against ${what}.` : "\n(no baseline to compare with)");
}

main().catch((err) => {
  cleanupNow();
  console.error(String(err?.message ?? err));
  process.exit(1);
});
