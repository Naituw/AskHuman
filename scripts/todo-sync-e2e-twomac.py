#!/usr/bin/env python3
"""Two real Macs against the CloudKit *Production* container: does a change on one Mac reach the other by itself?

The companion of todo-sync-e2e.py, which fakes two Macs on one. Here this Mac (A) and a second Mac (B, over ssh)
each run their own Developer ID signed helper with an independent fake HOME under a scratch root, so CloudKit sees
two devices and the push path, its latency and the idle-Mac behaviour are the real ones (plan P6, E1 / E2 / E3).
Nothing touches either Mac's real ~/.askhuman; the only thing written to the real home is the helper's log under
~/Library/Logs/AskHumanTodoSync, which `teardown` moves into TODO_SYNC_E2E2_OUT and removes.

Prerequisites:
  * the CloudKit schema is deployed to Production (CloudKit Dashboard, account owner only)
  * a Developer ID build:  HELPER_ALLOW_CLOUD_CHANGES=1 todo-sync-helper/scripts/build.sh devid   (contacts Apple)
  * an AskHuman built from this tree (./scripts/install.sh)
  * password-less ssh to B, B logged in to the same Apple ID and to a graphical session (the helper is started
    with `open`, as the product does)

usage:
  todo-sync-e2e-twomac.py setup              copy the binaries to both Macs, create the scratch repos, start both helpers
  todo-sync-e2e-twomac.py reset              blank slate without reinstalling: delete the zone, forget state and todos
  todo-sync-e2e-twomac.py run [test ...]     run tests (default: the whole list, see TESTS)
  todo-sync-e2e-twomac.py teardown           delete the zone in Production, stop the helpers, collect the logs, clean up

environment:
  TODO_SYNC_E2E2_REMOTE      ssh target of B (required)
  TODO_SYNC_E2E2_ROOT_A      scratch root on A (default /tmp/ah-ts2)
  TODO_SYNC_E2E2_ROOT_B      scratch root on B (default ~/AskHumanTodoSync-E2E there)
  TODO_SYNC_E2E2_ASKHUMAN    AskHuman binary (default <repo>/.askhuman-dev/bin/AskHuman)
  TODO_SYNC_E2E2_HELPER_APP  Developer ID helper (default <repo>/todo-sync-helper/build/out/devid/AskHumanTodoSync.app)
  TODO_SYNC_E2E2_WAIT_S      longest wait for one change to show up on the other Mac (default 1500)
  TODO_SYNC_E2E2_IDLE_MIN    minutes both Macs stay untouched before the idle test (default 10)
  TODO_SYNC_E2E2_OUT         where results and collected logs go (default /tmp/ah-ts2-out)
"""
import concurrent.futures
import datetime
import json
import os
import pwd
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REMOTE = os.environ.get("TODO_SYNC_E2E2_REMOTE", "")
ROOT_A = os.environ.get("TODO_SYNC_E2E2_ROOT_A", "/tmp/ah-ts2")
ROOT_B = os.environ.get("TODO_SYNC_E2E2_ROOT_B", "")
SRC_ASKHUMAN = os.environ.get("TODO_SYNC_E2E2_ASKHUMAN", f"{REPO}/.askhuman-dev/bin/AskHuman")
SRC_HELPER = os.environ.get("TODO_SYNC_E2E2_HELPER_APP", f"{REPO}/todo-sync-helper/build/out/devid/AskHumanTodoSync.app")
WAIT_S = int(os.environ.get("TODO_SYNC_E2E2_WAIT_S", "1500"))
IDLE_MIN = float(os.environ.get("TODO_SYNC_E2E2_IDLE_MIN", "10"))
OUT = os.environ.get("TODO_SYNC_E2E2_OUT", f"{tempfile.gettempdir()}/ah-ts2-out")
RESULTS = f"{OUT}/results.log"

# Short-lived shared connection: it makes the polling cheap, and it is gone 20 s after the last use, so an
# "idle" Mac is not held awake by an open ssh session during the idle test.
SSH_OPTS = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "ServerAliveInterval=15",
            "-o", "ControlMaster=auto", "-o", "ControlPath=/tmp/ah-ts2-ssh-%C", "-o", "ControlPersist=20"]
PROJECT = "proj1"
ORIGIN = "https://example.invalid/ah-ts2/proj1.git"
GIT_ENV = {**os.environ, "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"}


def log(msg):
    line = f"{time.strftime('%H:%M:%S')} {msg}"
    print(line, flush=True)
    os.makedirs(OUT, exist_ok=True)
    with open(RESULTS, "a") as f:
        f.write(line + "\n")


COUNTS = {"PASS": 0, "FAIL": 0}


def check(name, cond, detail=""):
    COUNTS["PASS" if cond else "FAIL"] += 1
    log(f"  {'PASS' if cond else 'FAIL'}  {name}" + (f"   [{detail}]" if detail and not cond else ""))
    return cond


def wait_for(pred, timeout, interval=1.0):
    t0 = time.time()
    while time.time() - t0 < timeout:
        v = pred()
        if v:
            return time.time() - t0
        time.sleep(interval)
    return None


class Mac:
    """One of the two Macs. `host` is None for this one; everything else goes through ssh."""

    def __init__(self, name, host, root):
        self.name, self.host, self.root = name, host, root

    home = property(lambda s: f"{s.root}/home")
    state = property(lambda s: f"{s.home}/.askhuman/state")
    side = property(lambda s: f"{s.state}/todo-sync")
    app = property(lambda s: f"{s.root}/bin/AskHumanTodoSync.app")
    helper_exe = property(lambda s: f"{s.app}/Contents/MacOS/AskHumanTodoSync")
    project_dir = property(lambda s: f"{s.home}/work/{PROJECT}")

    def run(self, script, timeout=180):
        if self.host:
            cmd = ["ssh", *SSH_OPTS, self.host, script]
            env = {**os.environ, "HOME": pwd.getpwuid(os.getuid()).pw_dir}
        else:
            # `open` hands the caller's environment to the app, and the helper passes it on to every tick it
            # spawns: ASKHUMAN_HOME (a Dev Instance) would turn all of them into silent no-ops.
            cmd, env = ["bash", "-c", script], {k: v for k, v in os.environ.items() if k != "ASKHUMAN_HOME"}
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env)
        return p.returncode, p.stdout, p.stderr

    def ah(self, *args, cwd=None, timeout=300):
        """AskHuman as this replica: fake HOME, never a Dev Instance, no keychain."""
        q = shlex.quote
        script = (f"cd {q(cwd or self.root)} && env -u ASKHUMAN_HOME HOME={q(self.home)} ASKHUMAN_NO_KEYCHAIN=1 "
                  f"{q(self.root + '/bin/AskHuman')} " + " ".join(q(a) for a in args))
        rc, out, err = self.run(script, timeout)
        return rc, out.strip(), err.strip()

    def status(self):
        rc, out, err = self.ah("todo-sync", "status", "--json")
        try:
            return json.loads(out)
        except Exception:
            return {"_raw": out or err}

    def snapshot(self):
        """(todo texts of the project, last-tick record) in one round trip."""
        q = shlex.quote
        rc, out, _ = self.run(f"cat {q(self.state + '/todos.json')} 2>/dev/null; printf '\\n@@@\\n'; cat {q(self.side + '/last-tick.json')} 2>/dev/null")
        todos_raw, _, tick_raw = out.partition("\n@@@\n")
        texts = []
        try:
            for key, entries in json.loads(todos_raw).get("projects", {}).items():
                if key.rsplit("/", 1)[-1] == PROJECT:
                    texts = [e["text"] for e in entries]
        except Exception:
            pass
        try:
            tick = json.loads(tick_raw)
        except Exception:
            tick = {}
        return texts, tick

    def helper_pid(self):
        rc, out, _ = self.run(f"pgrep -f {shlex.quote(self.helper_exe)} | head -n 1")
        return int(out.split()[0]) if out.split() else None

    def helper_start(self):
        q = shlex.quote
        env = [f"HOME={self.home}", f"ASKHUMAN_TODO_SYNC_DIR={self.side}", "ASKHUMAN_NO_KEYCHAIN=1"]
        script = (f"mkdir -p {q(self.side)} && open -g -n -a {q(self.app)} " + " ".join(f"--env {q(e)}" for e in env)
                  + f" --stdout {q(self.root + '/helper.out')} --stderr {q(self.root + '/helper.err')}")
        rc, out, err = self.run(script)
        if rc != 0:
            log(f"  info  {self.name}: open failed: {err.strip()[:200]}")
            return False
        return wait_for(self.helper_pid, 60, 1.0) is not None

    def helper_signal(self, sig):
        self.run(f"pid=$(pgrep -f {shlex.quote(self.helper_exe)} | head -n 1); [ -n \"$pid\" ] && kill -{sig} \"$pid\"; true")

    def helper_log_path(self):
        """The helper's log lives in the real home (not the fake one); ssh starts in the remote home."""
        if self.host is None:
            return f"{pwd.getpwuid(os.getuid()).pw_dir}/Library/Logs/AskHumanTodoSync/helper.log"
        return "Library/Logs/AskHumanTodoSync/helper.log"

    def helper_log(self, tail=600):
        rc, out, _ = self.run(f"tail -n {tail} {shlex.quote(self.helper_log_path())} 2>/dev/null")
        return out


def parse_events(log_text, name, since):
    """(epoch seconds, rest of line) of the helper log lines called `name` at or after `since`.

    A helper log line looks like `2026-10-27T16:54:17.246Z push ours=1`."""
    found = []
    for line in log_text.splitlines():
        parts = line.split(None, 2)
        if len(parts) < 2 or parts[1] != name:
            continue
        try:
            ts = datetime.datetime.fromisoformat(parts[0].replace("Z", "+00:00")).timestamp()
        except ValueError:
            continue
        if ts >= since:
            found.append((ts, parts[2] if len(parts) > 2 else ""))
    return found


def delivery_timeline(src_log, dst_log, t_start):
    """How one change travelled, from the two helper logs (epoch seconds, None when the step did not show).

    `saved`: the save on the source that produced the notification, `push`: the push notification reaching the
    destination, `fetched`: the destination's first fetch that brought changes, `reason`: the kind of tick that
    ended after it (push / poll / file / wake / started ...)."""
    saves = [ts for ts, rest in parse_events(src_log, "rpc_save", t_start - 1) if "records=0" not in rest]
    pushes = [ts for ts, _ in parse_events(dst_log, "push", t_start)]
    fetches = [ts for ts, rest in parse_events(dst_log, "rpc_fetch", t_start) if "changes=0" not in rest]
    reason = "?"
    for ts, rest in parse_events(dst_log, "tick_done", t_start):
        if fetches and ts >= fetches[0]:
            reason = next((w.split("=", 1)[1] for w in rest.split() if w.startswith("reason=")), "?")
            break
    arrival = pushes[0] if pushes else None
    saved = max((ts for ts in saves if arrival is None or ts <= arrival), default=None)
    return {"saved": saved, "push": arrival, "fetched": fetches[0] if fetches else None, "reason": reason}


A = Mac("A", None, ROOT_A)
B = Mac("B", REMOTE or None, ROOT_B)


def init_roots():
    global B
    if not REMOTE:
        sys.exit("set TODO_SYNC_E2E2_REMOTE to the ssh target of the second Mac")
    if not B.root:
        rc, out, err = Mac("B", REMOTE, "").run("echo $HOME")
        if rc != 0 or not out.strip():
            sys.exit(f"cannot reach {REMOTE} over ssh: {err.strip()[:200]}")
        B = Mac("B", REMOTE, f"{out.strip()}/AskHumanTodoSync-E2E")


# ---- setup / teardown ------------------------------------------------------------------------

def make_repo(m):
    q = shlex.quote
    script = (f"set -e; d={q(m.project_dir)}; mkdir -p \"$d\"; "
              f"[ -d \"$d/.git\" ] || {{ GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git init -q \"$d\"; "
              f"GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 git -C \"$d\" remote add origin {q(ORIGIN)}; }}")
    rc, out, err = m.run(script)
    if rc != 0:
        sys.exit(f"{m.name}: cannot create the scratch repository: {err.strip()[:200]}")


def cmd_setup():
    for src in (SRC_ASKHUMAN, SRC_HELPER):
        if not os.path.exists(src):
            sys.exit(f"missing {src}: see the prerequisites in the header of this script")
    init_roots()
    pack = tempfile.mkdtemp(prefix="ah-ts2-pack-")
    zip_path = f"{pack}/AskHumanTodoSync.zip"
    subprocess.run(["ditto", "-c", "-k", "--keepParent", SRC_HELPER, zip_path], check=True)
    scp_env = {**os.environ, "HOME": pwd.getpwuid(os.getuid()).pw_dir}
    for m in (A, B):
        m.run(f"mkdir -p {shlex.quote(m.root)}/bin {shlex.quote(m.home)}")
        if m.host is None:
            shutil.copy2(SRC_ASKHUMAN, f"{m.root}/bin/AskHuman.new")
            os.replace(f"{m.root}/bin/AskHuman.new", f"{m.root}/bin/AskHuman")
            shutil.rmtree(m.app, ignore_errors=True)
            subprocess.run(["ditto", "-x", "-k", zip_path, f"{m.root}/bin"], check=True)
        else:
            for src, dst in ((SRC_ASKHUMAN, f"{m.root}/bin/AskHuman.new"), (zip_path, f"{m.root}/AskHumanTodoSync.zip")):
                subprocess.run(["scp", "-q", *SSH_OPTS, src, f"{m.host}:{dst}"], check=True, env=scp_env)
            rc, out, err = m.run(f"set -e; cd {shlex.quote(m.root)}; mv -f bin/AskHuman.new bin/AskHuman; chmod +x bin/AskHuman; "
                                 f"rm -rf bin/AskHumanTodoSync.app; ditto -x -k AskHumanTodoSync.zip bin; rm -f AskHumanTodoSync.zip")
            if rc != 0:
                sys.exit(f"{m.name}: install failed: {err.strip()[:300]}")
        rc, out, err = m.run(f"codesign --verify --deep --strict {shlex.quote(m.app)} 2>&1 && echo signature-ok; "
                             f"{shlex.quote(m.root + '/bin/AskHuman')} --version 2>&1 | head -n 1")
        log(f"{m.name}: {out.strip().replace(chr(10), ' | ')}")
        make_repo(m)
    shutil.rmtree(pack, ignore_errors=True)
    for m in (A, B):
        if m.helper_pid() is None and not m.helper_start():
            sys.exit(f"{m.name}: the helper did not start; see {m.root}/helper.err")
        log(f"{m.name}: helper running (pid {m.helper_pid()})")
    log(f"scratch roots ready: A={A.root} B={B.root}")


def cmd_reset():
    """Blank slate on both Macs without reinstalling: delete the zone, forget the sync state and the todos."""
    init_roots()
    if A.status().get("enabled") and A.helper_pid():
        rc, out, err = A.ah("todo-sync", "clear-cloud", "--yes")
        log(f"clear-cloud (deletes the zone in Production): rc={rc} {out or err}")
    for m in (A, B):
        m.ah("todo-sync", "disable")
        names = " ".join(shlex.quote(f"{m.side}/{n}") for n in ("state.json", "enabled", "last-tick.json", "helper-launch", "helper.json", "todo-sync.lock"))
        m.run(f"rm -f {names} {shlex.quote(m.state + '/todos.json')} {shlex.quote(m.state + '/todos.lock')}")
    log("reset: cloud cleared, both replicas blank")


def cmd_teardown():
    init_roots()
    os.makedirs(OUT, exist_ok=True)
    if A.status().get("enabled") and A.helper_pid():
        rc, out, err = A.ah("todo-sync", "clear-cloud", "--yes")
        log(f"clear-cloud (deletes the zone in Production): rc={rc} {out or err}")
        B.ah("todo-sync", "tick", "--reason", "teardown")
        # Both Macs notice that the zone is gone: that is how the deletion shows from the outside.
        log(f"after clearing: A paused={A.status().get('paused')} B paused={B.status().get('paused')}")
    for m in (A, B):
        m.ah("todo-sync", "disable")
        m.helper_signal("TERM")
    time.sleep(2.0)
    for m in (A, B):
        # The helper's log lives in the real home, not in the fake one: keep it as evidence, then remove it.
        if m.host is None:
            real = pwd.getpwuid(os.getuid()).pw_dir
            src = f"{real}/Library/Logs/AskHumanTodoSync/helper.log"
            if os.path.exists(src):
                shutil.move(src, f"{OUT}/helper-{m.name}.log")
                try:
                    os.rmdir(os.path.dirname(src))
                except OSError:
                    pass
        else:
            scp_env = {**os.environ, "HOME": pwd.getpwuid(os.getuid()).pw_dir}
            r = subprocess.run(["scp", "-q", *SSH_OPTS, f"{m.host}:Library/Logs/AskHumanTodoSync/helper.log", f"{OUT}/helper-{m.name}.log"],
                               capture_output=True, text=True, env=scp_env)
            if r.returncode == 0:
                m.run("rm -f ~/Library/Logs/AskHumanTodoSync/helper.log; rmdir ~/Library/Logs/AskHumanTodoSync 2>/dev/null; true")
        m.run(f"rm -rf {shlex.quote(m.root)}")
    log(f"removed both scratch roots; logs are in {OUT}")


# ---- tests -----------------------------------------------------------------------------------

RUN = uuid.uuid4().hex[:5]


def text(tag):
    return f"{tag} {RUN}"


def is_synced_once(m):
    s = m.status()
    return bool(s.get("enabled")) and s.get("lastPullOkMs", 0) > 0 and (s.get("lastTick") or {}).get("outcome") in ("ok", "fresh")


def propagate(src, dst, texts, label):
    """Add `texts` on src through the CLI and wait for dst to show all of them, with no manual tick anywhere.

    Reports how long it took, when the source finished saving, when (and whether) a push notification reached the
    other Mac, and which kind of tick delivered the data there."""
    t_start = time.time()
    for t in texts:
        rc, out, err = src.ah("todo", "add", "--", t, cwd=src.project_dir)
        if rc != 0:
            check(f"{label}: add on {src.name}", False, out + err)
            return None

    def seen():
        got, _ = dst.snapshot()
        return all(t in got for t in texts)

    took = wait_for(seen, WAIT_S, 1.0)
    if took is None:
        check(f"{label}: {dst.name} shows the change within {WAIT_S}s", False, f"{src.name} lastTick={src.status().get('lastTick')}")
        return None
    # The timeline comes from the two helper logs.
    tl = delivery_timeline(src.helper_log(), dst.helper_log(), t_start)
    rel = lambda ts: None if ts is None else round(ts - t_start, 1)
    latency = None if tl["saved"] is None or tl["push"] is None else round(tl["push"] - tl["saved"], 1)
    log(f"  info  {label}: visible on {dst.name} +{took:.1f}s after the first add; {src.name} finished saving +{rel(tl['saved'])}s, "
        f"push notification reached {dst.name} +{rel(tl['push'])}s (push latency {latency}s), "
        f"{dst.name} fetched +{rel(tl['fetched'])}s; delivering tick reason={tl['reason']}")
    check(f"{label}: {dst.name} shows the change by itself", True)
    return took


def t_env():
    log("M0 both helpers are the Production build, running, and the Macs share one iCloud account")
    for m in (A, B):
        s = m.status()
        h = s.get("helper") or {}
        check(f"{m.name}: sync is supported here", s.get("supported") is True, json.dumps(s)[:200])
        log(f"  info  {m.name}: helper state={h.get('state')} version={h.get('version')} environment={h.get('environment')} problem={h.get('problem')}")


def t_first():
    log("M1 enable on both, A adds three todos, B must show them by itself (push, poll or file trigger)")
    for m in (A, B):
        rc, out, err = m.ah("todo-sync", "enable")
        check(f"{m.name}: enable accepted", rc == 0, out + err)
    for m in (A, B):
        took = wait_for(lambda: is_synced_once(m), 240, 2.0)
        check(f"{m.name}: first sync done", took is not None, json.dumps(m.status())[:300])
        if took is not None:
            h = m.status().get("helper") or {}
            log(f"  info  {m.name}: first sync took {took:.1f}s; helper environment={h.get('environment')} state={h.get('state')}")
    propagate(A, B, [text("alpha one"), text("alpha two"), text("alpha three")], "A -> B")


def t_reverse():
    log("M2 B adds two todos, A must show them by itself")
    propagate(B, A, [text("beta one"), text("beta two")], "B -> A")


def t_devices():
    log("M3 each Mac lists the other as a device")
    for m, other in ((A, B), (B, A)):
        s = m.status()
        rows = s.get("devices", [])
        log(f"  info  {m.name}: {len(rows)} device(s); this Mac flagged on {sum(1 for d in rows if d.get('thisMac'))}")
        check(f"{m.name}: two devices are known", len(s.get("devices", [])) >= 2, json.dumps(s.get("devices"))[:300])


def t_concurrent():
    log("M4 both Macs add at the same moment; both end with all of it")
    mine = [text("gamma A1"), text("gamma A2")]
    theirs = [text("gamma B1"), text("gamma B2")]

    def add_all(m, texts):
        for t in texts:
            m.ah("todo", "add", "--", t, cwd=m.project_dir)

    with concurrent.futures.ThreadPoolExecutor(2) as ex:
        list(ex.map(lambda args: add_all(*args), [(A, mine), (B, theirs)]))
    t0 = time.time()

    def converged():
        a, _ = A.snapshot()
        b, _ = B.snapshot()
        return all(t in a and t in b for t in mine + theirs) and sorted(a) == sorted(b)

    took = wait_for(converged, WAIT_S, 2.0)
    check("both Macs hold all four and the same set", took is not None, f"A={A.snapshot()[0]} B={B.snapshot()[0]}")
    if took is not None:
        log(f"  info  converged {took:.1f}s after the adds")
    a, _ = A.snapshot()
    check("nothing is duplicated", len(a) == len(set(a)), str(a))


def t_remove():
    log("M5 a removal on B reaches A; clearing the queue on A empties B")
    b_before, _ = B.snapshot()
    rc, out, err = B.ah("todo", "rm", "1", cwd=B.project_dir)
    b_after, _ = B.snapshot()
    removed = [t for t in b_before if t not in b_after]
    check("rm 1 on B removed one todo locally", rc == 0 and len(removed) == 1, out + err)
    if removed:
        took = wait_for(lambda: removed[0] not in A.snapshot()[0], WAIT_S, 1.0)
        check("A no longer shows it", took is not None)
        if took is not None:
            log(f"  info  removal visible on A after {took:.1f}s")
    rc, out, err = A.ah("todo", "clear", "--yes", cwd=A.project_dir)
    check("clear on A", rc == 0, out + err)
    took = wait_for(lambda: not B.snapshot()[0], WAIT_S, 1.0)
    check("B is empty too", took is not None, str(B.snapshot()[0]))
    if took is not None:
        log(f"  info  clear visible on B after {took:.1f}s")


def t_kill_helper():
    log("M6 B's helper is killed, A adds, B's helper comes back and catches up by itself")
    B.helper_signal("KILL")
    time.sleep(1.5)
    check("B's helper is gone", B.helper_pid() is None)
    rc, out, err = B.ah("todo-sync", "tick", "--reason", "matrix")
    check("a tick without a helper is a soft outcome naming the helper", rc == 0 and "helper-unavailable" in out, f"rc={rc} {out}{err}")
    t = text("delta while B was down")
    A.ah("todo", "add", "--", t, cwd=A.project_dir)
    time.sleep(5)
    check("B has not got it yet", t not in B.snapshot()[0])
    check("B's helper starts again", B.helper_start())
    t0 = time.time()
    took = wait_for(lambda: t in B.snapshot()[0], 300, 1.5)
    check("B catches up after the restart by itself", took is not None, f"lastTick={B.snapshot()[1]}")
    if took is not None:
        log(f"  info  caught up {time.time() - t0:.1f}s after the helper was started")


def t_idle():
    log(f"M7 both Macs stay untouched for {IDLE_MIN:g} minutes, then A adds one todo: how long until B shows it?")
    time.sleep(IDLE_MIN * 60)
    propagate(A, B, [text("epsilon after idle")], f"A -> B after {IDLE_MIN:g} idle minutes")


TESTS = {
    "env": t_env,
    "first": t_first,
    "reverse": t_reverse,
    "devices": t_devices,
    "concurrent": t_concurrent,
    "remove": t_remove,
    "kill-helper": t_kill_helper,
    "idle": t_idle,
}
DEFAULT_ORDER = ["env", "first", "reverse", "devices", "concurrent", "remove", "kill-helper", "idle"]

if __name__ == "__main__":
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        sys.exit(0)
    init_roots()
    if args[0] == "setup":
        cmd_setup()
    elif args[0] == "reset":
        cmd_reset()
    elif args[0] == "teardown":
        cmd_teardown()
    elif args[0] == "run":
        names = args[1:] or DEFAULT_ORDER
        unknown = [n for n in names if n not in TESTS]
        if unknown:
            sys.exit(f"unknown test(s) {unknown}; choose from {sorted(TESTS)}")
        for n in names:
            try:
                TESTS[n]()
            except Exception as e:  # keep going: report and move to the next test
                COUNTS["FAIL"] += 1
                log(f"  FAIL  {n} raised {type(e).__name__}: {e}")
        log(f"summary: {COUNTS['PASS']} PASS, {COUNTS['FAIL']} FAIL")
        sys.exit(1 if COUNTS["FAIL"] else 0)
    else:
        sys.exit(f"unknown command {args[0]!r}; see --help")
