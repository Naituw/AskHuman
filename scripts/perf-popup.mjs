#!/usr/bin/env node
// Deterministic popup-launch performance harness (spec docs/specs/popup-launch-performance.md §7,
// plan docs/plans/perf-harness-deterministic-mock-im.md).
//
// One brainless command — `node scripts/perf-popup.mjs` — runs a FIXED canonical scenario and
// gates against a FIXED baseline (docs/perf/baseline.json):
//   * No baseline yet  -> measures and writes it (first run bootstraps).
//   * Baseline exists   -> measures, prints cold+warm tables, exits non-zero if either end-to-end
//                          p90 regresses beyond the threshold.
//   * --update-baseline -> overwrite the baseline with the current numbers.
// There are deliberately NO flags that change WHAT is tested (runs / cold / config / channels are
// all built in) so the numbers are comparable across time and machines.
//
// The canonical scenario, per run:
//   * Throwaway $HOME + ASKHUMAN_HOME -> its own daemon/socket/perf.log/launchd label; never touches
//     the user's real daemon, and ASKHUMAN_NO_KEYCHAIN keeps it from reading/clobbering the real
//     OS keychain secrets.
//   * Local mock IM (scripts/perf-mock-im.mjs) with all four channels enabled (DingTalk/Feishu/
//     Telegram/Slack) pointed at it via config + ASKHUMAN_{DINGTALK,SLACK}_API_BASE. The mock adds
//     ~150ms to every connect/send so an "IM blocks the popup" regression shows up in e2e.
//   * Cold set  (daemon stopped before each run -> daemon cold start + IM reconnect every time;
//                this is where the IM-on-path delay lands today),
//     Warm set  (daemon kept hot, IM routers reused, popup prewarm OFF -> steady-state cold-spawn
//                popup latency; comparable to the pre-方案6 baseline) and
//     Hot set   (daemon kept hot, popup PREWARM ON -> a prewarmed helper is adopted per request, so
//                the WebView init/page-load/mount are pre-paid; measures the warm-path show->painted).
//                Only runs that actually hit the hot path (dmn.assigned) are aggregated.
//
// Prereq: run ./scripts/install.sh first so the on-disk binary carries the perf instrumentation,
// the keychain escape hatch and the IM base-URL overrides.
//
// Usage:
//   node scripts/perf-popup.mjs            # measure + compare-or-bootstrap baseline
//   node scripts/perf-popup.mjs --update-baseline
//   node scripts/perf-popup.mjs --keep-home    # (debug) keep the temp $HOME on exit

import { spawn, spawnSync } from "node:child_process";
import { readFileSync, writeFileSync, existsSync, mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { startMockIm } from "./perf-mock-im.mjs";

// ---- fixed scenario knobs (built in on purpose; not user-tunable) ----------
const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE_PATH = join(REPO_ROOT, "docs", "perf", "baseline.json");
const THRESHOLD_PCT = 20; // regression gate on e2e (spawn->painted) p90
const MOCK_DELAY_MS = 150; // injected IM connect/send latency (the regression probe)
const COLD_RUNS = 12, COLD_WARMUP = 2;
const WARM_RUNS = 24, WARM_WARMUP = 4;
const HOT_RUNS = 20, HOT_WARMUP = 4;
const RUN_TIMEOUT_MS = 30000;
const WARM_READY_TIMEOUT_MS = 5000; // max wait for a prewarmed helper to (re)appear before a hot run
const WARM_SETTLE_MS = 700; // after the warm process appears, let it finish mount + GuiWarmReady

// ---- arg parsing (only flags that don't change WHAT is tested) -------------

function parseArgs(argv) {
  const o = { updateBaseline: false, keepHome: false };
  for (const a of argv) {
    switch (a) {
      case "--update-baseline": o.updateBaseline = true; break;
      case "--keep-home": o.keepHome = true; break;
      case "-h": case "--help": printHelp(); process.exit(0); break;
      default:
        console.error(`unknown option: ${a}`);
        printHelp();
        process.exit(2);
    }
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

// ---- binary resolution -----------------------------------------------------

function resolveBin() {
  const candidates = [];
  if (process.env.ASKHUMAN_BIN) candidates.push(process.env.ASKHUMAN_BIN);
  candidates.push(join(homedir(), ".local", "bin", "AskHuman"));
  for (const c of candidates) {
    if (c && existsSync(c)) return c;
  }
  const which = spawnSync("which", ["AskHuman"], { encoding: "utf8" });
  if (which.status === 0 && which.stdout.trim()) return which.stdout.trim();
  const repo = join(REPO_ROOT, "src-tauri", "target", "release", "AskHuman");
  if (existsSync(repo)) return repo;
  console.error("could not locate the AskHuman binary; set $ASKHUMAN_BIN or run scripts/install.sh");
  process.exit(2);
}

// ---- display / lock guard --------------------------------------------------
// The popup paints in a real WebView; macOS pauses requestAnimationFrame (so fe.painted never
// fires and the popup never auto-dismisses) whenever the window isn't composited — i.e. the screen
// is locked or the display is asleep. We therefore refuse to run while locked and keep the display
// awake with `caffeinate` for the duration, so the numbers reflect a real on-screen paint.

/** macOS screen-lock state: "locked" | "unlocked" | "unknown" (non-macOS / unreadable). */
function screenLockState() {
  if (process.platform !== "darwin") return "unknown";
  const r = spawnSync("ioreg", ["-n", "Root", "-d1", "-r"], { encoding: "utf8" });
  if (r.status !== 0 || !r.stdout) return "unknown";
  const m = r.stdout.match(/"CGSSessionScreenIsLocked"\s*=\s*(Yes|No)/);
  return m ? (m[1] === "Yes" ? "locked" : "unlocked") : "unknown";
}

/** Abort with a clear message when the screen is locked (the popup can't paint -> data is invalid). */
function assertScreenUsable(where) {
  if (screenLockState() === "locked") {
    throw new Error(
      `screen is locked (${where}); the popup can't paint so latency is unmeasurable. ` +
        `Unlock the screen, keep it awake, and don't cover the popup, then re-run.`,
    );
  }
}

/** Keep the display awake for the whole run (macOS); returns a child to kill on teardown, or null. */
function startCaffeinate() {
  if (process.platform !== "darwin") return null;
  try {
    return spawn("caffeinate", ["-dimsu"], { stdio: "ignore" });
  } catch {
    return null;
  }
}

// ---- canonical isolated environment ----------------------------------------

/** Base env shared by every spawned process: isolated HOME, no keychain, IM base URLs -> mock.
 *  ASKHUMAN_HOME makes it an instance (own launchd label, no writes to the user's login items or
 *  agent hook files); overriding HOME alone is not enough to keep the real daemon out of reach. */
function childEnv(home, urls) {
  return {
    ...process.env,
    HOME: home,
    ASKHUMAN_HOME: join(home, ".askhuman"),
    ASKHUMAN_NO_KEYCHAIN: "1",
    ASKHUMAN_DINGTALK_API_BASE: urls.dingtalk,
    ASKHUMAN_SLACK_API_BASE: urls.slack,
  };
}

/** Write the canonical config.json (all four channels enabled, pointed at the mock).
 *  `prewarm` toggles 方案6 popup prewarm: OFF for cold/warm (measure cold-spawn), ON for the hot set. */
function writeCanonicalConfig(home, urls, prewarm) {
  const dir = join(home, ".askhuman");
  mkdirSync(dir, { recursive: true });
  const config = {
    general: { theme: "system", language: "en", popupPrewarm: !!prewarm },
    channels: {
      popup: { enabled: true },
      telegram: { enabled: true, botToken: "mock-bot", chatId: "1", apiBaseUrl: urls.telegram },
      dingding: { enabled: true, clientId: "mock-id", clientSecret: "mock-secret", userId: "u1" },
      feishu: { enabled: true, appId: "cli_mock", appSecret: "mock-secret", openId: "ou_mock", baseUrl: urls.feishu },
      slack: { enabled: true, botToken: "xoxb-mock", appToken: "xapp-mock", userId: "U1" },
      autoActivation: false,
    },
  };
  writeFileSync(join(dir, "config.json"), JSON.stringify(config, null, 2));
}

// ---- run helpers -----------------------------------------------------------

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Run a daemon control subcommand against the isolated HOME (blocking). */
function daemonCmd(bin, home, urls, args) {
  return spawnSync(bin, ["daemon", ...args], { stdio: "ignore", env: childEnv(home, urls) });
}

/** True iff a prewarmed popup helper for this HOME's daemon socket is currently running. */
function warmHelperRunning(home) {
  const socket = join(home, ".askhuman", "daemon.sock");
  // `--` so pgrep doesn't parse the pattern's leading dashes as flags; match the socket-scoped arg.
  const r = spawnSync("pgrep", ["-f", "--", `warm --endpoint ${socket}`], { encoding: "utf8" });
  return r.status === 0 && r.stdout.trim().length > 0;
}

/** Wait until a prewarmed helper (re)appears for this HOME, then let it settle (mount + GuiWarmReady). */
async function waitWarmReady(home) {
  const deadline = Date.now() + WARM_READY_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (warmHelperRunning(home)) {
      await sleep(WARM_SETTLE_MS);
      return true;
    }
    await sleep(100);
  }
  return false; // timed out: this run will likely cold-fall-back and be filtered out of the hot aggregate
}

/** Spawn one auto-dismissing AskHuman ask; resolves when the process exits. */
function runOnce(bin, home, urls) {
  return new Promise((resolve) => {
    // Stamp the spawn instant and inject it so the CLI records it under this run's perf_id; gives a
    // true end-to-end (spawn->painted) that includes OS process creation + dynamic-load before main.
    const spawnTs = Date.now();
    const child = spawn(
      bin,
      ["AskHuman perf probe", "-q", "perf probe (auto-dismiss)", "-o", "ok", "-o", "cancel"],
      {
        stdio: "ignore",
        env: {
          ...childEnv(home, urls),
          ASKHUMAN_PERF: "1",
          ASKHUMAN_PERF_AUTODISMISS: "1",
          ASKHUMAN_PERF_SPAWN_TS: String(spawnTs),
        },
      },
    );
    let done = false;
    const finish = (how) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      resolve(how);
    };
    const timer = setTimeout(() => {
      try { child.kill("SIGKILL"); } catch { /* ignore */ }
      finish("timeout");
    }, RUN_TIMEOUT_MS);
    child.on("exit", () => finish("exit"));
    child.on("error", () => finish("error"));
  });
}

// ---- perf.log parsing ------------------------------------------------------

// Named segments: [label, fromStage, toStage]. Indented labels are sub-segments of the line above.
// NOTE: gui.build_done marks when Tauri's `build()` returns (builder config); the heavy native
// window creation + first page load happen afterwards during run()/setup, so "window visible" /
// "page boot" are measured from gui.build_start / gui.show_recv.
const METRICS = [
  ["e2e+spawn (spawn->painted)", "spawn", "fe.painted"],
  ["  proc spawn (->cli.start)", "spawn", "cli.start"],
  ["e2e (cli.start->fe.painted)", "cli.start", "fe.painted"],
  ["cli (start->submit)", "cli.start", "cli.submit"],
  ["  detect", "cli.start", "cli.detect_done"],
  ["ipc (submit->dmn.recv)", "cli.submit", "dmn.submit_recv"],
  ["daemon (recv->spawned)", "dmn.submit_recv", "dmn.spawned"],
  ["daemon (recv->assigned/hot)", "dmn.submit_recv", "dmn.assigned"],
  ["  im_attach", "dmn.accepted", "dmn.im_done"],
  ["spawn->gui proc start", "dmn.spawned", "gui.start"],
  ["gui connect (start->show)", "gui.start", "gui.show_recv"],
  ["GUI total (show->painted)", "gui.show_recv", "fe.painted"],
  ["  tauri build()", "gui.build_start", "gui.build_done"],
  ["  window visible", "gui.build_start", "gui.win_show"],
  ["  page boot (->fe boot)", "gui.show_recv", "fe.bootstrap"],
  ["  frontend (boot->painted)", "fe.bootstrap", "fe.painted"],
  ["    popup_init", "fe.mounted", "fe.popup_init_done"],
];

const E2E_LABEL = "e2e+spawn (spawn->painted)";

/** Parse perf.log into { perfId -> { stage -> minTs } } keeping only cli.start in [from, to). */
function parsePerfLog(path, fromMs, toMs) {
  if (!existsSync(path)) return {};
  const groups = {};
  for (const line of readFileSync(path, "utf8").split("\n")) {
    if (!line) continue;
    const [tsStr, perfId, stage] = line.split("\t");
    const ts = Number(tsStr);
    if (!perfId || !stage || !Number.isFinite(ts)) continue;
    const g = (groups[perfId] ||= {});
    if (g[stage] === undefined || ts < g[stage]) g[stage] = ts;
  }
  // Bucket by cli.start within the window.
  const out = {};
  for (const [id, g] of Object.entries(groups)) {
    const s = g["cli.start"];
    if (s !== undefined && s >= fromMs && s < toMs) out[id] = g;
  }
  return out;
}

// ---- stats -----------------------------------------------------------------

function percentile(sorted, p) {
  if (sorted.length === 0) return null;
  if (sorted.length === 1) return sorted[0];
  const rank = (p / 100) * (sorted.length - 1);
  const lo = Math.floor(rank), hi = Math.ceil(rank);
  if (lo === hi) return sorted[lo];
  return sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo);
}

function summarize(values) {
  const v = values.filter((x) => Number.isFinite(x)).sort((a, b) => a - b);
  if (v.length === 0) return { count: 0, min: null, median: null, p90: null, max: null };
  return { count: v.length, min: v[0], median: percentile(v, 50), p90: percentile(v, 90), max: v[v.length - 1] };
}

/** Drop the `warmup` earliest invocations (by cli.start), then aggregate per-segment stats.
 *  `hotOnly` keeps only runs that actually hit the warm path (dmn.assigned present). */
function aggregate(groups, warmup, hotOnly = false) {
  let entries = Object.values(groups);
  if (warmup > 0) {
    entries = Object.entries(groups)
      .sort((a, b) => a[1]["cli.start"] - b[1]["cli.start"])
      .slice(warmup)
      .map(([, g]) => g);
  }
  let complete = entries.filter((g) => g["cli.start"] !== undefined && g["fe.painted"] !== undefined);
  if (hotOnly) complete = complete.filter((g) => g["dmn.assigned"] !== undefined);
  const metrics = {};
  for (const [label, from, to] of METRICS) {
    const vals = [];
    for (const g of complete) {
      if (g[from] !== undefined && g[to] !== undefined) vals.push(g[to] - g[from]);
    }
    metrics[label] = summarize(vals);
  }
  return { complete: complete.length, total: entries.length, metrics };
}

// ---- reporting -------------------------------------------------------------

const fmt = (n) => (n === null || n === undefined ? "  -" : n.toFixed(1).padStart(7));

function printTable(title, agg, baseAgg) {
  const baseMetrics = baseAgg?.metrics ?? null;
  console.log("");
  console.log(`== ${title} ==  ${agg.complete} complete / ${agg.total} runs` +
    (baseAgg ? `   (baseline: ${baseAgg.complete ?? "?"})` : ""));
  const head =
    "segment".padEnd(32) + "count".padStart(6) + "min".padStart(8) +
    "median".padStart(8) + "p90".padStart(8) + "max".padStart(8) +
    (baseMetrics ? "  base p90".padStart(10) + "   delta" : "");
  console.log(head);
  console.log("-".repeat(head.length));
  for (const [label] of METRICS) {
    const m = agg.metrics[label];
    let row =
      label.padEnd(32) + String(m.count).padStart(6) + fmt(m.min).padStart(8) +
      fmt(m.median).padStart(8) + fmt(m.p90).padStart(8) + fmt(m.max).padStart(8);
    if (baseMetrics) {
      const b = baseMetrics[label];
      if (b && b.p90 != null && b.p90 > 0 && m.p90 != null) {
        const deltaPct = ((m.p90 - b.p90) / b.p90) * 100;
        const sign = deltaPct >= 0 ? "+" : "";
        const flag = deltaPct > THRESHOLD_PCT ? " !" : "";
        row += fmt(b.p90).padStart(10) + `  ${sign}${deltaPct.toFixed(1)}%${flag}`;
      } else {
        row += fmt(b?.p90).padStart(10) + "   -";
      }
    }
    console.log(row);
  }
}

/** Compare one scenario's e2e p90 against baseline. Returns true if it regressed beyond threshold. */
function gate(title, agg, baseAgg) {
  if (!baseAgg) return false;
  const cur = agg.metrics[E2E_LABEL]?.p90;
  const base = baseAgg.metrics?.[E2E_LABEL]?.p90;
  if (cur == null || base == null || base <= 0) return false;
  const deltaPct = ((cur - base) / base) * 100;
  if (deltaPct > THRESHOLD_PCT) {
    console.error(`REGRESSION [${title}]: e2e p90 ${cur.toFixed(1)}ms vs baseline ${base.toFixed(1)}ms ` +
      `(+${deltaPct.toFixed(1)}% > ${THRESHOLD_PCT}%)`);
    return true;
  }
  console.log(`OK [${title}]: e2e p90 ${cur.toFixed(1)}ms vs baseline ${base.toFixed(1)}ms ` +
    `(${deltaPct >= 0 ? "+" : ""}${deltaPct.toFixed(1)}%, within ${THRESHOLD_PCT}%)`);
  return false;
}

// ---- main ------------------------------------------------------------------

async function runSet(label, bin, home, urls, runs, cold, hot = false) {
  process.stdout.write(
    `running ${label} set (${runs} runs${cold ? ", daemon cold each run" : ""}${hot ? ", prewarmed" : ""})...\n`,
  );
  for (let i = 0; i < runs; i++) {
    assertScreenUsable(`before ${label} run ${i + 1}`);
    if (cold) {
      daemonCmd(bin, home, urls, ["stop", "--force"]);
      await sleep(200);
    }
    // Hot set: wait for the (re)prewarmed helper to be ready so the request adopts it instead of
    // cold-falling-back. Runs that still miss are filtered out by the hot-only aggregate.
    if (hot) await waitWarmReady(home);
    const how = await runOnce(bin, home, urls);
    process.stdout.write(`\r  ${i + 1}/${runs} (${how})    `);
    if (how !== "exit") {
      process.stdout.write("\n");
      // A run that never exits means the popup never painted (auto-dismiss never fired). Re-check
      // lock to give a precise reason; either way the data would be invalid, so abort cleanly.
      const lock = screenLockState();
      throw new Error(
        `${label} run ${i + 1} did not complete (${how}); the popup never painted. ` +
          (lock === "locked"
            ? "Screen is locked — unlock it and re-run."
            : "Ensure the display is awake and the popup window is not occluded, then re-run."),
      );
    }
    await sleep(150);
  }
  process.stdout.write("\n");
}

async function main() {
  const o = parseArgs(process.argv.slice(2));
  const bin = resolveBin();
  // The popup must paint on a visible display; refuse to even start if the screen is locked.
  assertScreenUsable("at start");
  const caffeinate = startCaffeinate();
  const mock = await startMockIm({ delayMs: MOCK_DELAY_MS });
  const home = mkdtempSync(join(tmpdir(), "askhuman-perf-"));
  const perfLog = join(home, ".askhuman", "perf.log");
  // cold + warm sets measure cold-spawn popup latency → prewarm OFF.
  writeCanonicalConfig(home, mock.urls, false);

  console.log(`AskHuman:     ${bin}`);
  console.log(`isolated HOME: ${home}`);
  console.log(`mock IM:      127.0.0.1:${mock.port} (delay ${MOCK_DELAY_MS}ms, 4 channels)`);
  console.log(`baseline:     ${BASELINE_PATH}`);

  let exitCode = 0;
  try {
    // COLD: stop the daemon before each run -> daemon cold start + IM reconnect every time.
    daemonCmd(bin, home, mock.urls, ["stop", "--force"]);
    await sleep(200);
    const coldFrom = Date.now();
    await runSet("cold", bin, home, mock.urls, COLD_RUNS, true);
    await sleep(400);
    const coldTo = Date.now();

    // WARM: bring the daemon up once and keep it; IM routers stay hot after the first ask.
    daemonCmd(bin, home, mock.urls, ["stop", "--force"]);
    await sleep(200);
    daemonCmd(bin, home, mock.urls, ["start"]);
    await sleep(600);
    const warmFrom = Date.now();
    await runSet("warm", bin, home, mock.urls, WARM_RUNS, false);
    await sleep(400);
    const warmTo = Date.now();

    // HOT: popup prewarm ON (方案6). Restart the daemon with the new config so it preheats, then
    // each request adopts the prewarmed helper (WebView init pre-paid). Only hot-path runs counted.
    daemonCmd(bin, home, mock.urls, ["stop", "--force"]);
    await sleep(200);
    writeCanonicalConfig(home, mock.urls, true);
    daemonCmd(bin, home, mock.urls, ["start"]);
    await sleep(600);
    const hotFrom = Date.now();
    await runSet("hot", bin, home, mock.urls, HOT_RUNS, false, true);
    await sleep(400);
    const hotTo = Date.now();

    const coldAgg = aggregate(parsePerfLog(perfLog, coldFrom, coldTo), COLD_WARMUP);
    const warmAgg = aggregate(parsePerfLog(perfLog, warmFrom, warmTo), WARM_WARMUP);
    const hotAgg = aggregate(parsePerfLog(perfLog, hotFrom, hotTo), HOT_WARMUP, true);

    const baseline = !o.updateBaseline && existsSync(BASELINE_PATH)
      ? JSON.parse(readFileSync(BASELINE_PATH, "utf8"))
      : null;

    printTable("COLD (daemon+IM cold)", coldAgg, baseline?.cold);
    printTable("WARM (steady state, prewarm off)", warmAgg, baseline?.warm);
    printTable("HOT (方案6 prewarm, hot-path only)", hotAgg, baseline?.hot);
    console.log("");

    if (coldAgg.complete === 0 || warmAgg.complete === 0) {
      console.error("error: no complete invocations captured (is the installed binary instrumented?)");
      exitCode = 1;
    } else if (hotAgg.complete === 0) {
      console.error("error: no hot-path invocations captured (prewarm not adopted; check popupPrewarm/display)");
      exitCode = 1;
    } else if (o.updateBaseline || !existsSync(BASELINE_PATH)) {
      const action = o.updateBaseline ? "updated" : "bootstrapped";
      const out = {
        generatedAt: new Date().toISOString(),
        thresholdPct: THRESHOLD_PCT,
        mockDelayMs: MOCK_DELAY_MS,
        cold: { complete: coldAgg.complete, metrics: coldAgg.metrics },
        warm: { complete: warmAgg.complete, metrics: warmAgg.metrics },
        hot: { complete: hotAgg.complete, metrics: hotAgg.metrics },
      };
      mkdirSync(dirname(BASELINE_PATH), { recursive: true });
      writeFileSync(BASELINE_PATH, JSON.stringify(out, null, 2));
      console.log(`${action} baseline: ${BASELINE_PATH}`);
    } else {
      const regressed =
        gate("cold", coldAgg, baseline.cold) |
        gate("warm", warmAgg, baseline.warm) |
        gate("hot", hotAgg, baseline.hot);
      if (regressed) exitCode = 1;
    }
  } finally {
    daemonCmd(bin, home, mock.urls, ["stop", "--force"]);
    await mock.close();
    if (caffeinate) {
      try { caffeinate.kill(); } catch { /* ignore */ }
    }
    if (!o.keepHome) {
      try { rmSync(home, { recursive: true, force: true }); } catch { /* ignore */ }
    } else {
      console.log(`kept isolated HOME: ${home}`);
    }
  }
  process.exit(exitCode);
}

main().catch((e) => {
  console.error(`error: ${e?.message || e}`);
  process.exit(1);
});
