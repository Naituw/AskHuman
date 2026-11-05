// Offline tests for the ios-testflight skill: the change list (`changes.mjs`) and the rules of the
// helper's `notes` operation (`NotesRules.swift` inside the signed helper, `notes_*` in `helper.py`).
// Nothing here reads a credential, contacts Apple, or installs the helper.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { ChangesError, commitType, computeChanges } from "../.agents/skills/ios-testflight/scripts/changes.mjs";

const scriptsDir = fileURLToPath(new URL("../.agents/skills/ios-testflight/scripts/", import.meta.url));
const changesCli = join(scriptsDir, "changes.mjs");
const casesFile = join(scriptsDir, "notes-rules-cases.json");

// ── synthetic repositories ──────────────────────────────────────────────────────────────────────

const GIT_ENV = {
  ...process.env,
  GIT_CONFIG_GLOBAL: "/dev/null",
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_AUTHOR_NAME: "Test",
  GIT_AUTHOR_EMAIL: "test@example.com",
  GIT_COMMITTER_NAME: "Test",
  GIT_COMMITTER_EMAIL: "test@example.com",
  GIT_AUTHOR_DATE: "2026-11-04T12:00:00Z",
  GIT_COMMITTER_DATE: "2026-11-04T12:00:00Z",
};

function git(cwd, ...args) {
  return execFileSync("git", args, { cwd, encoding: "utf8", env: GIT_ENV }).trim();
}

function makeRepo(t) {
  const dir = mkdtempSync(join(tmpdir(), "ios-testflight-changes-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  git(dir, "init", "-q", "-b", "main");
  return dir;
}

let counter = 0;
function commit(dir, subject, files = { "ios/App.swift": null }) {
  for (const [path, content] of Object.entries(files)) {
    const full = join(dir, path);
    mkdirSync(join(full, ".."), { recursive: true });
    counter += 1;
    writeFileSync(full, content ?? `// change ${counter}\n`);
  }
  git(dir, "add", "-A");
  git(dir, "commit", "-q", "-m", subject);
  return git(dir, "rev-parse", "HEAD");
}

function reasonOf(fn) {
  try {
    fn();
  } catch (error) {
    assert.ok(error instanceof ChangesError, `expected a ChangesError, got ${error}`);
    return error;
  }
  assert.fail("expected a ChangesError");
}

// ── changes.mjs ─────────────────────────────────────────────────────────────────────────────────

test("commit types follow Conventional Commits and fall back to other", () => {
  assert.equal(commitType("feat(ios): add a switch"), "feat");
  assert.equal(commitType("Fix: capital letters count"), "fix");
  assert.equal(commitType("refactor!: drop the old model"), "refactor");
  assert.equal(commitType("Update the thing"), "other");
  assert.equal(commitType("feat:missing space"), "other");
});

test("without an ios-testflight tag the baseline is unknown", (t) => {
  const repo = makeRepo(t);
  commit(repo, "feat(ios): first");
  const error = reasonOf(() => computeChanges({ cwd: repo }));
  assert.equal(error.reason, "noBaseline");
});

test("lists only non-merge commits that touch ios/, oldest first, with their types", (t) => {
  const repo = makeRepo(t);
  const base = commit(repo, "feat(ios): build three");
  git(repo, "tag", "-a", "ios-testflight-3", "-m", "TestFlight 0.1.0 (3)");
  const fix = commit(repo, "fix(ios): crash on launch");
  commit(repo, "docs: unrelated", { "docs/note.md": "x\n" });
  const feat = commit(repo, "feat(ios): new switch");
  const loose = commit(repo, "Tweak spacing");
  const result = computeChanges({ cwd: repo });
  assert.deepEqual(result.commits.map((c) => c.hash), [fix, feat, loose]);
  assert.deepEqual(result.commits.map((c) => c.type), ["fix", "feat", "other"]);
  assert.deepEqual(result.commits.map((c) => c.subject), ["fix(ios): crash on launch", "feat(ios): new switch", "Tweak spacing"]);
  assert.equal(result.baseline.commit, base);
  assert.equal(result.baseline.build, 3);
  assert.equal(result.nextBuild, 4);
  assert.equal(result.empty, false);
  assert.equal(result.stat.files, 1);
  assert.equal(result.point, loose);
  assert.deepEqual(result.dropped, []);
});

test("the baseline is the largest build number, not the newest or the alphabetically last tag", (t) => {
  const repo = makeRepo(t);
  commit(repo, "feat(ios): a");
  git(repo, "tag", "ios-testflight-10");
  commit(repo, "feat(ios): b");
  git(repo, "tag", "-a", "ios-testflight-2", "-m", "an older number tagged later");
  git(repo, "tag", "ios-testflight-notes");
  const result = computeChanges({ cwd: repo });
  assert.equal(result.baseline.tag, "ios-testflight-10");
  assert.equal(result.nextBuild, 11);
  assert.deepEqual(result.commits.map((c) => c.subject), ["feat(ios): b"]);
});

test("a baseline at the build point gives an empty list", (t) => {
  const repo = makeRepo(t);
  commit(repo, "feat(ios): only");
  git(repo, "tag", "ios-testflight-1");
  commit(repo, "chore: elsewhere", { "scripts/x.mjs": "x\n" });
  const result = computeChanges({ cwd: repo });
  assert.equal(result.empty, true);
  assert.deepEqual(result.commits, []);
  assert.equal(result.stat.summary, "no changes under ios/");
});

test("merge commits are not listed but the work they bring in is", (t) => {
  const repo = makeRepo(t);
  commit(repo, "feat(ios): base");
  git(repo, "tag", "ios-testflight-1");
  git(repo, "switch", "-q", "-c", "side");
  const side = commit(repo, "feat(ios): on the side", { "ios/Side.swift": null });
  git(repo, "switch", "-q", "main");
  const mainWork = commit(repo, "fix(ios): on main", { "ios/Main.swift": null });
  git(repo, "merge", "-q", "--no-ff", "-m", "chore: merge side", "side");
  const result = computeChanges({ cwd: repo });
  assert.deepEqual(new Set(result.commits.map((c) => c.hash)), new Set([side, mainWork]));
  assert.ok(result.commits.every((c) => c.subject !== "chore: merge side"));
});

test("a baseline that is not an ancestor of the build point is refused", (t) => {
  const repo = makeRepo(t);
  commit(repo, "feat(ios): base");
  git(repo, "switch", "-q", "-c", "elsewhere");
  commit(repo, "feat(ios): not in main", { "ios/Else.swift": null });
  git(repo, "tag", "ios-testflight-4");
  git(repo, "switch", "-q", "main");
  commit(repo, "feat(ios): main moves on", { "ios/Main.swift": null });
  assert.equal(reasonOf(() => computeChanges({ cwd: repo })).reason, "notAncestor");
});

test("building from HEAD needs a clean working tree; an earlier point does not", (t) => {
  const repo = makeRepo(t);
  const first = commit(repo, "feat(ios): one");
  git(repo, "tag", "ios-testflight-1");
  const second = commit(repo, "feat(ios): two");
  writeFileSync(join(repo, "scratch.txt"), "untracked\n");
  const error = reasonOf(() => computeChanges({ cwd: repo }));
  assert.equal(error.reason, "dirtyTree");
  assert.deepEqual(error.details.paths, ["?? scratch.txt"]);
  const earlier = computeChanges({ cwd: repo, point: first });
  assert.equal(earlier.empty, true);
  assert.deepEqual(earlier.dropped.map((c) => c.hash), [second]);
  writeFileSync(join(repo, "ios/App.swift"), "// edited but not committed\n");
  assert.equal(reasonOf(() => computeChanges({ cwd: repo })).reason, "dirtyTree");
});

test("leaving commits out: the dropped set must equal the one the user chose", (t) => {
  const repo = makeRepo(t);
  commit(repo, "feat(ios): base");
  git(repo, "tag", "ios-testflight-1");
  const wanted = commit(repo, "feat(ios): wanted", { "ios/A.swift": null });
  const heldBack = commit(repo, "feat(ios): held back", { "ios/B.swift": null });
  const alsoLater = commit(repo, "fix(ios): later fix", { "ios/C.swift": null });

  // The held-back commit is followed by one the user did not exclude: the build point would drop both.
  const interleaved = reasonOf(() => computeChanges({ cwd: repo, point: wanted, expectDropped: [heldBack.slice(0, 8)] }));
  assert.equal(interleaved.reason, "droppedMismatch");
  assert.deepEqual(interleaved.details.extra.map((c) => c.hash), [alsoLater]);

  // Excluding exactly what the build point drops works, with abbreviated hashes too.
  const exact = computeChanges({ cwd: repo, point: wanted, expectDropped: [heldBack.slice(0, 8), alsoLater] });
  assert.deepEqual(exact.commits.map((c) => c.hash), [wanted]);
  assert.deepEqual(exact.dropped.map((c) => c.hash), [heldBack, alsoLater]);

  // A commit the user wants out but that stays in the package is reported as missing.
  const missing = reasonOf(() => computeChanges({ cwd: repo, point: heldBack, expectDropped: [wanted, alsoLater] }));
  assert.equal(missing.reason, "droppedMismatch");
  assert.deepEqual(missing.details.missing, [wanted.slice(0, 7)]);

  // Dropping nothing while expecting something is a mismatch as well.
  assert.equal(reasonOf(() => computeChanges({ cwd: repo, expectDropped: [alsoLater] })).reason, "droppedMismatch");
});

test("a build point outside the tip's history cannot leave commits out", (t) => {
  const repo = makeRepo(t);
  commit(repo, "feat(ios): base");
  git(repo, "tag", "ios-testflight-1");
  git(repo, "switch", "-q", "-c", "side");
  const side = commit(repo, "feat(ios): side", { "ios/Side.swift": null });
  git(repo, "switch", "-q", "main");
  commit(repo, "feat(ios): main", { "ios/Main.swift": null });
  assert.equal(reasonOf(() => computeChanges({ cwd: repo, point: side })).reason, "pointNotBeforeTip");
});

test("unknown refs and malformed hashes are refused with badRef", (t) => {
  const repo = makeRepo(t);
  commit(repo, "feat(ios): base");
  git(repo, "tag", "ios-testflight-1");
  assert.equal(reasonOf(() => computeChanges({ cwd: repo, point: "no-such-ref" })).reason, "badRef");
  assert.equal(reasonOf(() => computeChanges({ cwd: repo, tip: "no-such-ref" })).reason, "badRef");
  assert.equal(reasonOf(() => computeChanges({ cwd: repo, expectDropped: ["xyz"] })).reason, "badRef");
});

test("the command line reports in text and JSON and maps failures to exit codes", (t) => {
  const repo = makeRepo(t);
  commit(repo, "feat(ios): base");
  git(repo, "tag", "ios-testflight-5");
  const pick = commit(repo, "fix(ios): after five");
  const run = (...args) => spawnSync("node", [changesCli, ...args], { cwd: repo, encoding: "utf8" });

  const text = run();
  assert.equal(text.status, 0, text.stderr);
  assert.match(text.stdout, /Baseline: ios-testflight-5/);
  assert.match(text.stdout, /Next build number: 6/);
  assert.ok(text.stdout.includes(`${pick.slice(0, 7)} 2026-11-04 fix      fix(ios): after five`), text.stdout);

  const json = run("--json");
  assert.equal(json.status, 0, json.stderr);
  assert.equal(JSON.parse(json.stdout).nextBuild, 6);

  writeFileSync(join(repo, "dirty.txt"), "x\n");
  const dirty = run("--json");
  assert.equal(dirty.status, 1);
  assert.equal(JSON.parse(dirty.stdout).error.reason, "dirtyTree");
  assert.match(run().stderr, /changes: dirtyTree:/);

  assert.equal(run("--bogus").status, 2);
  assert.equal(run("--point").status, 2);
  assert.equal(run("--help").status, 0);
});

// ── helper.py rules (the launcher's side of the notes operation) ────────────────────────────────

const PYTHON_CASES = String.raw`
import json, sys
sys.dont_write_bytecode = True
sys.path.insert(0, sys.argv[1])
import helper

def expand(value):
    if isinstance(value, dict):
        if "$repeat" in value:
            return value["$repeat"] * value["$count"] + value.get("$suffix", "")
        return {key: expand(item) for key, item in value.items()}
    if isinstance(value, list):
        return [expand(item) for item in value]
    return value

cases = json.load(open(sys.argv[2], encoding="utf-8"))
failures, checks = [], 0

def outcome(call):
    try:
        return ("ok", call())
    except helper.NotesRejected as error:
        return ("error", error.reason)

for case in cases["canonicalize"]:
    checks += 1
    got = outcome(lambda: helper.notes_canonicalize(expand(case["input"])))
    want = ("error", case["error"]) if "error" in case else ("ok", expand(case["expect"]))
    if got != want:
        failures.append("canonicalize: %s: got %r" % (case["name"], got[1] if got[0] == "error" else "a different text"))

for case in cases["build"]:
    checks += 1
    accepted = outcome(lambda: helper.notes_validate_build(case["value"]))[0] == "ok"
    if accepted != case["ok"]:
        failures.append("build %r: expected ok=%s" % (case["value"], case["ok"]))

for case in cases["request"]:
    checks += 1
    got = outcome(lambda: helper.notes_request(expand(case["request"])))
    want = ("error", case["error"]) if "error" in case else ("ok", expand(case["expect"]))
    if got != want:
        failures.append("request: %s: got %r" % (case["name"], got))
    if got[0] == "ok":
        # What the launcher puts on the wire must be accepted, unchanged, by the helper's own validation.
        wire = helper.notes_wire(got[1])
        if outcome(lambda: helper.notes_request(wire)) != ("ok", got[1]):
            failures.append("wire form of %s is not accepted again: %r" % (case["name"], wire))
        if got[1]["mode"] != "write" and "texts" in wire:
            failures.append("wire form of %s must not carry texts" % case["name"])

print(json.dumps({"checks": checks, "failures": failures}))
`;

test("helper.py reproduces every shared notes-rules case", () => {
  const run = spawnSync("python3", ["-B", "-c", PYTHON_CASES, scriptsDir, casesFile], { encoding: "utf8" });
  assert.equal(run.status, 0, run.stderr);
  const report = JSON.parse(run.stdout);
  assert.deepEqual(report.failures, []);
  const cases = JSON.parse(readFileSync(casesFile, "utf8"));
  assert.equal(report.checks, cases.canonicalize.length + cases.build.length + cases.request.length);
});

function notesDryRun(t, args, files = {}) {
  const dir = mkdtempSync(join(tmpdir(), "ios-testflight-notes-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const resolved = args.map((arg) => (files[arg] === undefined ? arg : join(dir, arg)));
  for (const [name, content] of Object.entries(files)) writeFileSync(join(dir, name), content);
  // A config path that cannot exist proves the dry run needs neither credentials nor the installed helper.
  return spawnSync("python3", ["-B", join(scriptsDir, "helper.py"), "notes", "--config", join(dir, "absent", "config.json"), ...resolved], { encoding: "utf8" });
}

test("notes --dry-run validates and describes a write without starting the helper", (t) => {
  const zh = "构建 7 · 源码 abc1234\n• 新增";
  const en = "Build 7 · source abc1234\n• Added";
  const run = notesDryRun(t, ["--build", "7", "--mode", "write", "--zh", "zh.txt", "--en", "en.txt", "--dry-run"], {
    // CRLF and trailing spaces and newlines are normalized away before the length is measured.
    "zh.txt": "构建 7 · 源码 abc1234\r\n• 新增  \n",
    "en.txt": "Build 7 · source abc1234\n• Added\n\n",
  });
  assert.equal(run.status, 0, run.stderr);
  const report = JSON.parse(run.stdout);
  assert.equal(report.dryRun, true);
  assert.equal(report.mode, "write");
  assert.equal(report.build, "7");
  // A JavaScript string's length is its UTF-16 unit count, the unit the limit is measured in.
  assert.deepEqual(report.locales, { "en-US": { length: en.length }, "zh-Hans": { length: zh.length } });
  assert.ok(report.wouldCall.some((call) => call.startsWith("PATCH ")));
  assert.ok(!report.wouldCall.some((call) => call.startsWith("DELETE ")));
});

test("notes --dry-run describes read and probe, and probe is the only mode that deletes", (t) => {
  const read = JSON.parse(notesDryRun(t, ["--build", "6", "--mode", "read", "--dry-run"]).stdout);
  assert.deepEqual(read.locales, {});
  assert.ok(read.wouldCall.every((call) => call.startsWith("GET ")));
  const probe = JSON.parse(notesDryRun(t, ["--build", "6", "--mode", "probe", "--dry-run"]).stdout);
  assert.ok(probe.wouldCall.some((call) => call.startsWith("DELETE ")));
});

test("notes refuses what the helper would refuse, before anything is started", (t) => {
  const tooLong = notesDryRun(t, ["--build", "7", "--mode", "write", "--en", "long.txt", "--dry-run"], { "long.txt": "a".repeat(4001) });
  assert.equal(tooLong.status, 1);
  assert.match(tooLong.stderr, /tooLong/);
  const control = notesDryRun(t, ["--build", "7", "--mode", "write", "--en", "tab.txt", "--dry-run"], { "tab.txt": "a\tb" });
  assert.equal(control.status, 1);
  assert.match(control.stderr, /control/);
  const badBuild = notesDryRun(t, ["--build", "07", "--mode", "read", "--dry-run"]);
  assert.equal(badBuild.status, 1);
  assert.match(badBuild.stderr, /badBuild/);
  const notUtf8 = notesDryRun(t, ["--build", "7", "--mode", "write", "--en", "bad.txt", "--dry-run"], { "bad.txt": Buffer.from([0xff, 0xfe, 0x41]) });
  assert.notEqual(notUtf8.status, 0);
});

test("notes argument combinations are checked", (t) => {
  assert.equal(notesDryRun(t, ["--mode", "read", "--dry-run"]).status, 2, "build is required");
  assert.equal(notesDryRun(t, ["--build", "7", "--dry-run"]).status, 2, "mode is required");
  assert.equal(notesDryRun(t, ["--build", "7", "--mode", "write", "--dry-run"]).status, 2, "a write needs text");
  assert.equal(notesDryRun(t, ["--build", "7", "--mode", "read", "--en", "en.txt", "--dry-run"], { "en.txt": "x" }).status, 2, "only a write takes text");
  const status = spawnSync("python3", ["-B", join(scriptsDir, "helper.py"), "status", "--build", "7"], { encoding: "utf8" });
  assert.equal(status.status, 2, "notes options are rejected for other actions");
});

// ── NotesRules.swift (inside the signed helper) ─────────────────────────────────────────────────

const canCompileSwift = process.platform === "darwin" && spawnSync("xcrun", ["--find", "swiftc"], { stdio: "ignore" }).status === 0;

test("NotesRules.swift passes its own tests and the shared cases", { skip: !canCompileSwift && "needs macOS with swiftc" }, (t) => {
  const dir = mkdtempSync(join(tmpdir(), "ios-testflight-swift-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const binary = join(dir, "notes-rules-tests");
  const compile = spawnSync("xcrun", ["swiftc", "-parse-as-library", join(scriptsDir, "NotesRules.swift"), join(scriptsDir, "NotesRulesTests.swift"), "-o", binary], { encoding: "utf8" });
  assert.equal(compile.status, 0, compile.stderr);
  const run = spawnSync(binary, [casesFile], { encoding: "utf8" });
  assert.equal(run.status, 0, run.stdout + run.stderr);
  assert.match(run.stdout, /NotesRulesTests: \d+ checks, 0 failed/);
});

test("the signed helper's sources still type-check together", { skip: !canCompileSwift && "needs macOS with swiftc" }, () => {
  const check = spawnSync("xcrun", ["swiftc", "-typecheck", "-parse-as-library", ...["TestFlightHelper.swift", "NotesRules.swift", "ExternalStatus.swift", "DistributionRules.swift", "ExternalTesting.swift"].map(name => join(scriptsDir, name))], { encoding: "utf8" });
  assert.equal(check.status, 0, check.stderr);
});
