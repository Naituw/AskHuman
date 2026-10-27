#!/usr/bin/env python3
"""Two-replica acceptance run for iCloud todo sync against the CloudKit *Development* environment.

Simulates two Macs on one Mac: replica A and B are two independent fake HOMEs under a scratch root (never the
real ~/.askhuman). One dev-signed helper serves both: A is started with the helper's own triggers (file
watcher, poll, push); B is driven by manual `todo-sync tick`s over a symlink to A's helper socket (the helper
is a stateless transport). CloudKit therefore sees ONE device, so push latency between two Macs still needs
the real two-Mac run (docs/plans/todo-sync.md, E1).

Prerequisites (see the "真实 iCloud" item under "验证" in docs/overview-todo-sync.md):
  * a dev-signed helper:  HELPER_ALLOW_CLOUD_CHANGES=1 todo-sync-helper/scripts/build.sh dev   (contacts Apple)
  * an AskHuman built from this tree (./scripts/install.sh); the replicas run a copy of it with ASKHUMAN_HOME unset,
    so they are not Dev Instances (those never enable sync)
  * this Mac signed in to iCloud; the container opened once in the CloudKit Dashboard if `ensure` is rejected

usage:
  todo-sync-e2e.py setup              copy the binaries, create replicas A and B, start the helper
  todo-sync-e2e.py reset              blank slate: clear the Development zone and both replicas
  todo-sync-e2e.py run [test ...]     run tests (default: the whole matrix, see TESTS)
  todo-sync-e2e.py teardown           delete the Development zone, stop the helper, remove the scratch root

environment:
  TODO_SYNC_E2E_ROOT         scratch root (default /tmp/ah-ts)
  TODO_SYNC_E2E_ASKHUMAN     AskHuman binary (default <repo>/.askhuman-dev/bin/AskHuman)
  TODO_SYNC_E2E_HELPER_APP   dev-signed helper (default <repo>/todo-sync-helper/build/out/dev/AskHumanTodoSync.app)
  TODO_SYNC_E2E_KILLS        iterations of the kill -9 test (default 14; the helper is killed every 7th)
  TODO_SYNC_E2E_SEED         random seed of the kill -9 test (default 7)
"""
import fcntl
import json
import os
import random
import re
import shutil
import signal
import socket
import subprocess
import sys
import time
import uuid

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ROOT = os.environ.get("TODO_SYNC_E2E_ROOT", "/tmp/ah-ts")
SRC_ASKHUMAN = os.environ.get("TODO_SYNC_E2E_ASKHUMAN", f"{REPO}/.askhuman-dev/bin/AskHuman")
SRC_HELPER = os.environ.get("TODO_SYNC_E2E_HELPER_APP", f"{REPO}/todo-sync-helper/build/out/dev/AskHumanTodoSync.app")
BIN = f"{ROOT}/bin/AskHuman"
# The scratch repositories never read the user's own git configuration (hooks, signing, aliases).
GIT_ENV = {**os.environ, "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"}

HELPER_BIN = f"{ROOT}/bin/AskHumanTodoSync.app/Contents/MacOS/AskHumanTodoSync"
RESULTS = f"{ROOT}/results.log"
# macOS resolves /tmp to /private/tmp; the app keys projects by the canonical path.
REAL_ROOT = os.path.realpath(ROOT)


def _run_id():
    """Appended to todo texts so that reruns never trip over leftovers; `reset` starts a new one."""
    path = f"{ROOT}/run-id"
    try:
        with open(path) as f:
            return f.read().strip()
    except FileNotFoundError:
        value = uuid.uuid4().hex[:4]
        os.makedirs(ROOT, exist_ok=True)
        with open(path, "w") as f:
            f.write(value)
        return value


RUN = _run_id()


def H(r):
    return f"{ROOT}/{r}"


def STATE(r):
    return f"{H(r)}/.askhuman/state"


def SIDE(r):
    return f"{STATE(r)}/todo-sync"


def PDIR(r, name):
    """Directory of project `name` on replica r (as the app keys it: /tmp is /private/tmp)."""
    return f"{REAL_ROOT}/{r}/work/{name}"


def env_for(r):
    e = {k: v for k, v in os.environ.items() if k != "ASKHUMAN_HOME"}
    e.update(HOME=H(r), ASKHUMAN_NO_KEYCHAIN="1")
    return e


def log(msg):
    line = f"{time.strftime('%H:%M:%S')} {msg}"
    print(line, flush=True)
    with open(RESULTS, "a") as f:
        f.write(line + "\n")


def ah(r, *args, cwd="/tmp", timeout=300):
    """Run AskHuman as replica r. Returns (rc, stdout, stderr, seconds)."""
    t0 = time.time()
    p = subprocess.run([BIN, *args], cwd=cwd, env=env_for(r), capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout.strip(), p.stderr.strip(), time.time() - t0


def tick(r, reason="matrix", retries=6):
    """One full tick; waits out `busy` (the helper's own trigger may be ticking A)."""
    for _ in range(retries):
        rc, out, err, secs = ah(r, "todo-sync", "tick", "--reason", reason)
        if "busy" in out:
            time.sleep(1.5)
            continue
        return rc, (out or err), secs
    return rc, (out or err), secs


def status(r):
    rc, out, err, _ = ah(r, "todo-sync", "status", "--json")
    try:
        return json.loads(out)
    except Exception:
        return {"_raw": out or err}


def ensure_project(r, name, origin=None):
    d = f"{H(r)}/work/{name}"
    if not os.path.isdir(f"{d}/.git"):
        os.makedirs(d, exist_ok=True)
        subprocess.run(["git", "init", "-q", d], check=True, env=GIT_ENV)
        subprocess.run(["git", "-C", d, "remote", "add", "origin", origin or f"https://example.invalid/ah-ts/{name}.git"], check=True, env=GIT_ENV)
    return d


# ---- todos.json access under the same flock the app uses ------------------------------------

def todos_path(r):
    return f"{STATE(r)}/todos.json"


def read_todos(r):
    try:
        with open(todos_path(r)) as f:
            d = json.load(f)
    except FileNotFoundError:
        d = {}
    d.setdefault("projects", {})
    d.setdefault("history", {})
    return d


def mutate(r, fn):
    os.makedirs(STATE(r), exist_ok=True)
    with open(f"{STATE(r)}/todos.lock", "a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            data = read_todos(r)
            fn(data)
            data["projects"] = {k: v for k, v in data["projects"].items() if v}
            data["history"] = {k: v for k, v in data["history"].items() if v}
            tmp = todos_path(r) + f".tmp-{uuid.uuid4()}"
            with open(tmp, "w") as f:
                json.dump(data, f, indent=2)
            os.rename(tmp, todos_path(r))
        finally:
            fcntl.flock(lock, fcntl.LOCK_UN)


def now_ms():
    return int(time.time() * 1000)


def add(r, name, text, auto=False):
    """Add through the real CLI (the path the user's agents take)."""
    args = ["todo", "add"] + (["--auto"] if auto else []) + ["--", text]
    rc, out, err, _ = ah(r, *args, cwd=PDIR(r, name))
    assert rc == 0, f"todo add failed: {out} {err}"


def queue(r, name):
    return read_todos(r)["projects"].get(PDIR(r, name), [])


def hist(r, name):
    return read_todos(r)["history"].get(PDIR(r, name), [])


def find(entries, text):
    for e in entries:
        if e["text"] == text:
            return e
    return None


def edit_text(r, name, old, new):
    def fn(d):
        e = find(d["projects"].get(PDIR(r, name), []), old)
        assert e, f"{old!r} not in queue of {r}/{name}"
        e["text"] = new
    mutate(r, fn)


def set_auto_flag(r, name, text, auto):
    def fn(d):
        e = find(d["projects"].get(PDIR(r, name), []), text)
        assert e, f"{text!r} not in queue of {r}/{name}"
        if auto:
            e["auto"] = True
        else:
            e.pop("auto", None)
    mutate(r, fn)


def reorder(r, name, texts):
    def fn(d):
        q = d["projects"].get(PDIR(r, name), [])
        by = {e["text"]: e for e in q}
        assert set(by) == set(texts), f"reorder needs the full set: {sorted(by)} vs {sorted(texts)}"
        d["projects"][PDIR(r, name)] = [by[t] for t in texts]
    mutate(r, fn)


def complete(r, name, text):
    def fn(d):
        k = PDIR(r, name)
        q = d["projects"].get(k, [])
        e = find(q, text)
        assert e, f"{text!r} not in queue of {r}/{name}"
        q.remove(e)
        done = {kk: vv for kk, vv in e.items() if kk != "auto"}
        done["doneAtMs"] = now_ms()
        d["history"].setdefault(k, []).insert(0, done)
    mutate(r, fn)


def restore(r, name, text):
    def fn(d):
        k = PDIR(r, name)
        h = d["history"].get(k, [])
        e = find(h, text)
        assert e, f"{text!r} not in history of {r}/{name}"
        h.remove(e)
        back = {kk: vv for kk, vv in e.items() if kk != "doneAtMs"}
        d["projects"].setdefault(k, []).append(back)
    mutate(r, fn)


def delete_pending(r, name, text):
    def fn(d):
        q = d["projects"].get(PDIR(r, name), [])
        e = find(q, text)
        assert e, f"{text!r} not in queue of {r}/{name}"
        q.remove(e)
    mutate(r, fn)


def delete_done(r, name, text):
    def fn(d):
        h = d["history"].get(PDIR(r, name), [])
        e = find(h, text)
        assert e, f"{text!r} not in history of {r}/{name}"
        h.remove(e)
    mutate(r, fn)


def clear_history(r, name):
    mutate(r, lambda d: d["history"].pop(PDIR(r, name), None))


# ---- views ----------------------------------------------------------------------------------

# A second directory of the same repository on one Mac (made by the worktree test) is another replica of the
# same project, not another project: it is checked on its own, never compared across Macs.
WORKTREE_SUFFIX = "-wt"


def label(k):
    return k if k.startswith("icloud:") else k.rsplit("/", 1)[-1]


def view(r, with_auto=False):
    """What a human sees: per project, the ordered queue (text, auto) and the history texts.

    `auto` (the lightning flag) never leaves a Mac (spec T5), so it is reported as False unless asked for."""
    d = read_todos(r)
    out = {}
    for k, v in d["projects"].items():
        if k.endswith(WORKTREE_SUFFIX):
            continue
        out.setdefault(label(k), {})["q"] = [(e["text"], bool(e.get("auto")) if with_auto else False) for e in v]
    for k, v in d["history"].items():
        if k.endswith(WORKTREE_SUFFIX):
            continue
        out.setdefault(label(k), {})["h"] = sorted(e["text"] for e in v)
    return out


def fmt(v):
    return json.dumps(v, ensure_ascii=False, sort_keys=True)


def converge(max_rounds=4):
    """Tick both replicas alternately until their views agree."""
    last = None
    for i in range(max_rounds):
        for r in ("A", "B"):
            tick(r)
        a, b = view("A"), view("B")
        if a == b:
            return True, a
        last = (a, b)
    return False, last


def check(name, cond, detail=""):
    log(f"  {'PASS' if cond else 'FAIL'}  {name}" + (f"   [{detail}]" if detail and not cond else ""))
    return cond


def wait_for(pred, timeout, interval=0.25):
    t0 = time.time()
    while time.time() - t0 < timeout:
        v = pred()
        if v:
            return time.time() - t0
        time.sleep(interval)
    return None


# ---- helper control -----------------------------------------------------------------------------

HELPER_JSON = f"{SIDE('A')}/helper.json"


def helper_pid():
    out = subprocess.run(["pgrep", "-f", f"{ROOT}/bin/AskHumanTodoSync.app/Contents/MacOS/AskHumanTodoSync"], capture_output=True, text=True).stdout.split()
    return int(out[0]) if out else None


def helper_auto(enabled):
    """Switch the helper's own triggers (file watcher, poll, push) on or off; RPC keeps working."""
    try:
        with open(HELPER_JSON) as f:
            cfg = json.load(f)
    except FileNotFoundError:
        return
    if cfg.get("enabled") == enabled:
        return
    cfg["enabled"] = enabled
    tmp = HELPER_JSON + ".tmp"
    with open(tmp, "w") as f:
        json.dump(cfg, f, indent=2)
    os.rename(tmp, HELPER_JSON)
    time.sleep(2.0)  # let the helper's file watcher apply it


def rpc(op, **fields):
    """One call to the helper's socket (A's), JSON in / JSON out."""
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(90)
        s.connect(f"{SIDE('A')}/helper.sock")
        s.sendall((json.dumps({"id": 1, "op": op, **fields}) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
        s.close()
        return json.loads(buf.decode())
    except Exception as e:  # no helper, refused, timed out
        return {"ok": False, "_error": f"{type(e).__name__}: {e}"}


def start_helper():
    """The dev-signed helper for replica A. HOME is A's, so the ticks it spawns see replica A only."""
    env = env_for("A")
    env["ASKHUMAN_TODO_SYNC_DIR"] = SIDE("A")
    out = open(f"{ROOT}/helper-A.out", "a")
    subprocess.Popen([HELPER_BIN], env=env, stdout=out, stderr=subprocess.STDOUT, start_new_session=True)
    return wait_for(lambda: rpc("hello").get("ok") is True, 90, 1.0) is not None


def restart_helper():
    return start_helper()


# ---- tests --------------------------------------------------------------------------------------

def t_rpc():
    log("T1 RPC round trip through the real helper")
    hello = rpc("hello")
    check("hello ok, development, iCloud account available", hello.get("ok") and hello.get("environment") == "development" and hello.get("accountStatus") == "available", str(hello))
    ens = rpc("ensure")
    check("ensure creates / finds the zone", ens.get("ok") is True, str(ens))
    page = rpc("fetch")
    check("fetch answers (page shape)", page.get("ok") is True and "changes" in page, str(page)[:300])
    st = rpc("status")
    check("status reports apns registered", st.get("apns") == "registered", str(st))


def setup_replicas():
    for r in ("A", "B"):
        ensure_project(r, "proj1")
        os.makedirs(SIDE(r), exist_ok=True)
    sock_b = f"{SIDE('B')}/helper.sock"
    if not os.path.islink(sock_b):
        if os.path.exists(sock_b):
            os.remove(sock_b)
        os.symlink(f"{SIDE('A')}/helper.sock", sock_b)
    for r in ("A", "B"):
        if not os.path.exists(f"{SIDE(r)}/enabled"):
            rc, out, err, _ = ah(r, "todo-sync", "enable")
            assert rc == 0, f"enable {r}: {out} {err}"
    # the "enable" ticks run detached; wait for their report
    for r in ("A", "B"):
        wait_for(lambda r=r: os.path.exists(f"{SIDE(r)}/last-tick.json"), 60)
    time.sleep(2)


def cmd_setup():
    """Copy the binaries into the scratch root, create replicas A and B, start the helper."""
    for src in (SRC_ASKHUMAN, SRC_HELPER):
        if not os.path.exists(src):
            sys.exit(f"missing {src}: see the prerequisites in the header of this script")
    os.makedirs(f"{ROOT}/bin", exist_ok=True)
    shutil.copy2(SRC_ASKHUMAN, BIN)
    shutil.rmtree(f"{ROOT}/bin/AskHumanTodoSync.app", ignore_errors=True)
    subprocess.run(["cp", "-R", SRC_HELPER, f"{ROOT}/bin/AskHumanTodoSync.app"], check=True)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", f"{ROOT}/bin/AskHumanTodoSync.app"], check=True)
    for r in ("A", "B"):
        os.makedirs(SIDE(r), exist_ok=True)
        ensure_project(r, "proj1")
    if rpc("hello").get("ok") is not True:
        if not start_helper():
            sys.exit(f"the helper did not answer; see {ROOT}/helper-A.out and ~/Library/Logs/AskHumanTodoSync/helper.log")
    log(f"helper: {json.dumps(rpc('hello'))}")
    log(f"scratch root {ROOT} ready (replicas A and B, helper pid {helper_pid()})")


def cmd_reset():
    """Back to a blank slate: clear the Development zone, forget both replicas' sync state and todos."""
    if os.path.exists(f"{SIDE('A')}/enabled") and rpc("hello").get("ok") is True:
        helper_auto(False)
        rc, out, err, _ = ah("A", "todo-sync", "clear-cloud", "--yes")
        log(f"clear-cloud: rc={rc} {out or err}")
    for r in ("A", "B"):
        for name in ("state.json", "enabled", "last-tick.json", "helper-launch", "helper.json", "todo-sync.lock"):
            try:
                os.remove(f"{SIDE(r)}/{name}")
            except FileNotFoundError:
                pass
        for name in ("todos.json", "todos.lock"):
            try:
                os.remove(f"{STATE(r)}/{name}")
            except FileNotFoundError:
                pass
    try:
        os.remove(f"{ROOT}/run-id")
    except FileNotFoundError:
        pass
    log("reset: cloud cleared, replicas blank")


def cmd_teardown():
    """Delete the Development zone, stop the helper, remove the scratch root."""
    if os.path.exists(f"{SIDE('A')}/enabled") and rpc("hello").get("ok") is True:
        helper_auto(False)
        rc, out, err, _ = ah("A", "todo-sync", "clear-cloud", "--yes")
        log(f"clear-cloud: rc={rc} {out or err}")
    pid = helper_pid()
    if pid:
        os.kill(pid, signal.SIGTERM)
    time.sleep(1.0)
    shutil.rmtree(ROOT, ignore_errors=True)
    print(f"removed {ROOT}")


def t_basic():
    log("T2 first sync and basic propagation (A -> B)")
    setup_replicas()
    for t in ("alpha one", "alpha two", "alpha three"):
        add("A", "proj1", f"{t} {RUN}")
    rc, out, secs = tick("A")
    check("A tick pushes", rc == 0 and "pushed=" in out and not re.search(r"pushed=0\b", out), f"{out} ({secs:.1f}s)")
    rc, out, secs = tick("B")
    check("B tick pulls", rc == 0 and "pulled" in out, f"{out} ({secs:.1f}s)")
    # B has the directory but no key yet: reading it materializes (and takes over a virtual key)
    ah("B", "todo", "list", cwd=PDIR("B", "proj1"))
    qa = [e["text"] for e in queue("A", "proj1")]
    qb = [e["text"] for e in queue("B", "proj1")]
    check("B shows A's todos in the same order", qa == qb and len(qa) >= 3, f"A={qa} B={qb}")
    check("no virtual key left on B", not any(k.startswith("icloud:") for k in read_todos("B")["projects"]), fmt(view("B")))


def t_ops():
    log("T3 edit / reorder / complete / restore / delete propagate")
    helper_auto(False)
    try:
        ok, _ = converge()
        check("start converged", ok)
        # different fields of the same entity edited concurrently (text on A, position on B), no sync in between
        edit_text("A", "proj1", f"alpha two {RUN}", f"alpha two {RUN} (edited on A)")
        cur = [e["text"] for e in queue("B", "proj1")]
        moved = [f"alpha two {RUN}"] + [t for t in cur if t != f"alpha two {RUN}"]
        reorder("B", "proj1", moved)
        ok, v = converge()
        qa = [x[0] for x in view("A")["proj1"]["q"]]
        qb = [x[0] for x in view("B")["proj1"]["q"]]
        check("text edit on A and move on B both survive on both", ok and qa[0] == f"alpha two {RUN} (edited on A)" and qa == qb, fmt(v))
        # the lightning flag is local-only (spec T5): setting it on B never reaches A
        set_auto_flag("B", "proj1", f"alpha three {RUN}", True)
        converge()
        check("the auto flag stays on the Mac that set it", (f"alpha three {RUN}", True) in view("B", True)["proj1"]["q"] and (f"alpha three {RUN}", False) in view("A", True)["proj1"]["q"], fmt([view("A", True), view("B", True)]))
        # reorder on B
        cur = [e["text"] for e in queue("B", "proj1")]
        new = [cur[-1]] + cur[:-1]
        reorder("B", "proj1", new)
        ok, v = converge()
        check("reorder on B reaches A", ok and [e["text"] for e in queue("A", "proj1")] == new, fmt(v))
        # complete on A
        victim = new[1]
        complete("A", "proj1", victim)
        ok, v = converge()
        check("complete on A appears in B's history", ok and victim in view("B")["proj1"].get("h", []) and victim not in [x[0] for x in view("B")["proj1"]["q"]], fmt(v))
        # restore on B
        restore("B", "proj1", victim)
        ok, v = converge()
        check("restore on B brings it back on A", ok and victim in [x[0] for x in view("A")["proj1"]["q"]] and victim not in view("A")["proj1"].get("h", []), fmt(v))
        # delete a pending one on A
        gone = [e["text"] for e in queue("A", "proj1")][0]
        delete_pending("A", "proj1", gone)
        ok, v = converge()
        check("delete on A disappears on B", ok and gone not in [x[0] for x in view("B")["proj1"]["q"]], fmt(v))
        # add on B then delete a history item on A
        add("B", "proj1", "history victim")
        complete("B", "proj1", "history victim")
        ok, v = converge()
        check("B's completed todo is in A's history", ok and "history victim" in view("A")["proj1"].get("h", []), fmt(v))
        delete_done("A", "proj1", "history victim")
        ok, v = converge()
        check("deleting a history entry on A removes it on B", ok and "history victim" not in view("B")["proj1"].get("h", []), fmt(v))
    finally:
        helper_auto(True)


def t_concurrent():
    log("T4 concurrent edits of the same field converge to one winner")
    helper_auto(False)
    try:
        ok, _ = converge()
        base = queue("A", "proj1")[0]["text"]
        edit_text("A", "proj1", base, base + " / A")
        time.sleep(0.05)
        edit_text("B", "proj1", base, base + " / B")
        ok, v = converge()
        a = [x[0] for x in view("A")["proj1"]["q"]]
        b = [x[0] for x in view("B")["proj1"]["q"]]
        check("both replicas agree", ok and a == b, fmt(v))
        winner = [t for t in a if t.startswith(base)]
        check("exactly one of the two edits survives and nothing is duplicated", len(winner) == 1 and winner[0] in (base + " / A", base + " / B"), str(winner))
        log(f"  info  winner={winner}")
        # an add on both sides at the same time is two todos, not one
        add("A", "proj1", "twin from A")
        add("B", "proj1", "twin from B")
        ok, v = converge()
        names = [x[0] for x in view("A")["proj1"]["q"]]
        check("two concurrent adds both survive", ok and "twin from A" in names and "twin from B" in names, fmt(v))
    finally:
        helper_auto(True)


def t_history_clear():
    log("T5 clearing history is global")
    helper_auto(False)
    try:
        add("A", "proj1", "to be history 1")
        add("A", "proj1", "to be history 2")
        complete("A", "proj1", "to be history 1")
        complete("A", "proj1", "to be history 2")
        ok, v = converge()
        check("B sees A's history", ok and {"to be history 1", "to be history 2"} <= set(view("B")["proj1"].get("h", [])), fmt(v))
        clear_history("B", "proj1")
        ok, v = converge()
        check("clearing history on B empties it on A as well", ok and not view("A")["proj1"].get("h"), fmt(v))
        # nothing resurrects it on later ticks
        converge()
        check("history stays empty after more ticks", not view("A")["proj1"].get("h") and not view("B")["proj1"].get("h"))
    finally:
        helper_auto(True)


def t_virtual():
    log("T6 project without a local directory (virtual key) and takeover")
    helper_auto(False)
    try:
        ensure_project("A", "proj2")
        add("A", "proj2", "cloud only one")
        add("A", "proj2", "cloud only two")
        tick("A")
        rc, out, _ = tick("B")
        v = read_todos("B")
        vkeys = [k for k in v["projects"] if k.startswith("icloud:")]
        check("B holds the project under a virtual key", len(vkeys) == 1 and len(v["projects"][vkeys[0]]) == 2, fmt(view("B")))
        projects = status("B")
        log(f"  info  B status after pull: {json.dumps(projects)[:300]}")
        # a directory with the same origin appears on B: reading it takes the todos over
        ensure_project("B", "proj2")
        rc, out, err, _ = ah("B", "todo", "list", cwd=PDIR("B", "proj2"))
        v = read_todos("B")
        check("takeover moved them to the real directory", PDIR("B", "proj2") in v["projects"] and len(v["projects"][PDIR("B", "proj2")]) == 2 and not any(k.startswith("icloud:") for k in v["projects"]), fmt(view("B")) + " :: " + out)
        ok, vv = converge()
        check("A and B agree afterwards", ok, fmt(vv))
    finally:
        helper_auto(True)


def set_poll_minutes(minutes):
    with open(HELPER_JSON) as f:
        cfg = json.load(f)
    cfg["pollMinutes"] = minutes
    cfg["enabled"] = True
    tmp = HELPER_JSON + ".tmp"
    with open(tmp, "w") as f:
        json.dump(cfg, f, indent=2)
    os.rename(tmp, HELPER_JSON)
    time.sleep(2.0)


def t_push():
    log("T7 A's helper catches up on B's uploads by itself (push, else the 1-minute poll used here)")
    log("  info  one helper serves both replicas, i.e. CloudKit sees ONE device: a push for B's save may be suppressed;")
    log("  info  the real push latency needs two Macs (plan E1). Here: does A catch up on its own, and did a push arrive?")
    set_poll_minutes(1)
    try:
        st0 = rpc("status")
        samples = []
        for i in range(4):
            text = f"push probe {i} {uuid.uuid4().hex[:6]}"
            push_before = rpc("status").get("lastPushMs", 0)
            add("B", "proj1", text)
            rc, out, secs = tick("B")
            t_done = time.time()
            got = wait_for(lambda: find(queue("A", "proj1"), text) is not None, 150, 0.2)
            push_after = rpc("status").get("lastPushMs", 0)
            if got is None:
                samples.append(None)
                log(f"  info  sample {i}: A did not see it within 150 s (B tick {secs:.1f}s)")
            else:
                lat = time.time() - t_done
                samples.append(lat)
                log(f"  info  sample {i}: B tick {secs:.1f}s; A saw it {lat:.1f}s later; push received since: {push_after != push_before}")
            time.sleep(10)
        seen = [s for s in samples if s is not None]
        check("A catches up on every upload without being asked", len(seen) == len(samples), str(samples))
        if seen:
            seen.sort()
            log(f"  info  catch-up after B's upload: min {seen[0]:.1f}s median {seen[len(seen)//2]:.1f}s max {seen[-1]:.1f}s")
        st1 = rpc("status")
        log(f"  info  helper status: ticksRun {st0.get('ticksRun')} -> {st1.get('ticksRun')}, lastTickReason={st1.get('lastTickReason')}, lastPushMs={st1.get('lastPushMs')}, apns={st1.get('apns')}")
    finally:
        set_poll_minutes(10)


def t_kill():
    log("T8 kill -9 loop on B's ticks and on the helper; nothing lost or duplicated")
    helper_auto(False)
    try:
        ok, _ = converge()
        created = set()
        rng = random.Random(int(os.environ.get("TODO_SYNC_E2E_SEED", "7")))
        for i in range(int(os.environ.get("TODO_SYNC_E2E_KILLS", "14"))):
            op = rng.choice(["add", "add", "edit", "complete", "delete"])
            q = [e["text"] for e in queue("B", "proj1")]
            if op == "add" or not q:
                text = f"kill {i} {uuid.uuid4().hex[:5]}"
                add("B", "proj1", text)
                created.add(text)
            elif op == "edit":
                old = rng.choice(q)
                new = f"{old} +e{i}"
                edit_text("B", "proj1", old, new)
                if old in created:
                    created.discard(old)
                    created.add(new)
            elif op == "complete":
                complete("B", "proj1", rng.choice(q))
            else:
                victim = rng.choice(q)
                delete_pending("B", "proj1", victim)
                created.discard(victim)
            # start a tick and kill it at a random moment
            p = subprocess.Popen([BIN, "todo-sync", "tick", "--reason", "kill"], cwd="/tmp", env=env_for("B"), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            time.sleep(rng.uniform(0.05, 1.8))
            if p.poll() is None:
                p.send_signal(signal.SIGKILL)
            p.wait()
            if i % 7 == 6:
                pid = helper_pid()
                if pid:
                    os.kill(pid, signal.SIGKILL)
                    log(f"  info  helper killed (pid {pid}); restarting")
                    time.sleep(1.0)
                    restart_helper()
                    helper_auto(False)
        ok, v = converge(max_rounds=6)
        check("A and B converge after the abuse", ok, fmt(v))
        names_a = [x[0] for x in view("A")["proj1"]["q"]]
        check("no duplicates in the queue", len(names_a) == len(set(names_a)), str(names_a))
        missing = [t for t in created if t not in names_a and t not in view("A")["proj1"].get("h", [])]
        check("every surviving created todo is still there", not missing, str(missing))
    finally:
        helper_auto(True)


def t_idle():
    log("T9 idle cost of the helper (5 minutes, triggers on)")
    helper_auto(True)
    pid = helper_pid()
    if not pid:
        check("helper running", False)
        return

    def sample():
        out = subprocess.run(["ps", "-o", "%cpu=,rss=,time=", "-p", str(pid)], capture_output=True, text=True).stdout.split()
        return out

    t0 = sample()
    log(f"  info  start: %cpu={t0[0]} rss={int(t0[1])/1024:.0f} MiB cputime={t0[2]}")
    time.sleep(300)
    t1 = sample()
    log(f"  info  after 5 min: %cpu={t1[0]} rss={int(t1[1])/1024:.0f} MiB cputime={t1[2]}")


def t_clear():
    log("T10 clear iCloud data, pause and recovery on both replicas")
    helper_auto(False)
    try:
        ok, _ = converge()
        rc, out, err, _ = ah("A", "todo-sync", "clear-cloud", "--yes")
        check("clear-cloud on A succeeds", rc == 0, out + err)
        sa = status("A")
        check("A is paused afterwards", bool(sa.get("paused")), json.dumps(sa)[:300])
        rc, out, secs = tick("B")
        sb = status("B")
        log(f"  info  B tick after clear: {out}; paused={sb.get('paused')}")
        check("B notices the cloud was cleared (paused or soft message)", bool(sb.get("paused")) or "paused" in out or "cleared" in out.lower(), json.dumps(sb)[:300])
        before_a = view("A")
        rc, out, err, _ = ah("A", "todo-sync", "restart-from-local")
        check("restart-from-local on A", rc == 0, out + err)
        tick("A")
        sa = status("A")
        check("A is syncing again", not sa.get("paused"), json.dumps(sa)[:300])
        check("A's local todos untouched by clearing", view("A") == before_a)
        rc, out, err, _ = ah("B", "todo-sync", "restart-from-local")
        tick("B")
        ok, v = converge()
        check("both replicas agree again after both restarted from local", ok, fmt(v))
    finally:
        helper_auto(True)


def t_offline():
    log("E5/E9 both replicas edit while the helper is gone, then catch up")
    helper_auto(False)
    try:
        ok, _ = converge()
        check("start converged", ok)
        pid = helper_pid()
        os.kill(pid, signal.SIGKILL)
        time.sleep(1.0)
        add("A", "proj1", "offline edit on A")
        add("B", "proj1", "offline edit on B")
        rc, out, secs = tick("B")
        check("tick without a helper is a soft outcome naming the helper", rc == 0 and "helper-unavailable" in out, f"rc={rc} {out}")
        check("local reads keep working", find(queue("B", "proj1"), "offline edit on B") is not None)
        st = status("B")
        log(f"  info  B status without helper: helper={json.dumps(st.get('helper'))[:200]}")
        check("restart helper", restart_helper())
        helper_auto(False)
        ok, v = converge()
        names = [x[0] for x in view("A")["proj1"]["q"]]
        check("both offline edits arrived on both", ok and "offline edit on A" in names and "offline edit on B" in names, fmt(v))
    finally:
        helper_auto(True)


def t_worktree():
    log("E6 several directories of the same repository on one Mac")
    helper_auto(False)
    try:
        main = ensure_project("A", "proj1")
        wt = f"{H('A')}/work/proj1-wt"
        if not os.path.isdir(wt):
            subprocess.run(["git", "-C", main, "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "--allow-empty", "-q", "-m", "init"], check=True, env=GIT_ENV)
            subprocess.run(["git", "-C", main, "worktree", "add", "-q", "-b", "wt-branch", wt], check=True, env=GIT_ENV)
        ok, _ = converge()
        # first read of the new directory materializes the project's todos there
        wt_real = PDIR("A", "proj1-wt")
        rc, out, err, _ = ah("A", "todo", "list", cwd=wt_real)
        main_q = [e["text"] for e in queue("A", "proj1")]
        wt_q = [e["text"] for e in read_todos("A")["projects"].get(wt_real, [])]
        check("the new worktree shows the repository's todos on first read", main_q == wt_q and len(main_q) > 0, f"main={main_q} wt={wt_q} :: {out}")
        # an add in the worktree reaches the main directory and the other Mac
        rc, out, err, _ = ah("A", "todo", "add", "--", "added in the worktree", cwd=wt_real)
        ok, v = converge()
        names_main = [x[0] for x in view("A")["proj1"]["q"]]
        check("the worktree's add reaches the main directory", "added in the worktree" in names_main, fmt(view("A")))
        check("and the other Mac", "added in the worktree" in [x[0] for x in view("B")["proj1"]["q"]], fmt(view("B")))
        rc, out, err, _ = ah("A", "todo", "rm", "1", cwd=wt_real)
        ok, v = converge()
        check("a removal in the worktree reaches the main directory too", [x[0] for x in view("A")["proj1"]["q"]] == [x[0] for x in view("B")["proj1"]["q"]] and len(view("A")["proj1"]["q"]) == len(names_main) - 1, fmt(view("A")))
    finally:
        helper_auto(True)


def t_account():
    log("E11 account change (state.json carries a different account hash) pauses sync; restart-from-local recovers")
    helper_auto(False)
    try:
        ok, _ = converge()
        p = f"{SIDE('B')}/state.json"
        with open(p) as f:
            st = json.load(f)
        real = st.get("accountHash")
        st["accountHash"] = "0123456789abcdef"
        with open(p, "w") as f:
            json.dump(st, f)
        before = view("B")
        add("B", "proj1", "added while paused")
        rc, out, secs = tick("B")
        sb = status("B")
        check("B pauses with accountChanged", "accountChanged" in json.dumps(sb.get("paused")) or "accountChanged" in out, out + " " + json.dumps(sb)[:200])
        check("local todos untouched and still editable while paused", find(queue("B", "proj1"), "added while paused") is not None)
        rc, out, err, _ = ah("B", "todo-sync", "restart-from-local")
        check("restart-from-local accepted", rc == 0, out + err)
        ok, v = converge()
        check("B syncs again and both agree", ok and "added while paused" in [x[0] for x in view("A")["proj1"]["q"]], fmt(v))
        sb = status("B")
        check("the recorded account hash is the real one again", not sb.get("paused"), json.dumps(sb)[:200])
    finally:
        helper_auto(True)


def t_rebuild():
    log("E12 deleting the sidecar metadata and enabling again rebuilds from iCloud without losing or doubling todos")
    helper_auto(False)
    try:
        ok, _ = converge()
        check("start converged", ok)
        before = view("B")
        ids_before = sorted(e["id"] for e in queue("B", "proj1"))
        # partial damage first: the marker is there but the state file is not. The next tick rebuilds it.
        old_device = json.load(open(f"{SIDE('B')}/state.json")).get("deviceId")
        os.remove(f"{SIDE('B')}/state.json")
        rc, out, secs = tick("B")
        log(f"  info  tick with the marker but without state.json: rc={rc} {out}")
        check("a tick rebuilds the missing state instead of idling", rc == 0 and "disabled" not in out and os.path.exists(f"{SIDE('B')}/state.json"), out)
        ok, v = converge(6)
        check("still converged, no duplicates, nothing lost after the rebuild", ok and view("B") == before and len({x[0] for x in view("A")["proj1"]["q"]}) == len(view("A")["proj1"]["q"]), fmt(v))
        new_device = json.load(open(f"{SIDE('B')}/state.json")).get("deviceId")
        check("the rebuilt state is a new device", old_device != new_device)
        # the documented path: delete the whole sidecar state (marker included), then enable again
        for name in ("enabled", "state.json", "last-tick.json"):
            try:
                os.remove(f"{SIDE('B')}/{name}")
            except FileNotFoundError:
                pass
        rc, out, err, _ = ah("B", "todo-sync", "enable")
        check("enable again accepted", rc == 0, out + err)
        wait_for(lambda: os.path.exists(f"{SIDE('B')}/last-tick.json"), 60)
        time.sleep(2)
        ok, v = converge(6)
        check("both agree and B's list is what it was", ok and view("B") == before, fmt(v))
        names = [x[0] for x in view("A")["proj1"]["q"]]
        check("no duplicates anywhere", len(names) == len(set(names)), str(names))
        ids_after = sorted(e["id"] for e in queue("B", "proj1"))
        check("the same todos (same ids) as before", ids_before == ids_after, f"{len(ids_before)} vs {len(ids_after)}")
    finally:
        helper_auto(True)


def t_final_cleanup():
    log("T99 delete the Development test data (clear-cloud) and detach")
    helper_auto(False)
    rc, out, err, _ = ah("A", "todo-sync", "clear-cloud", "--yes")
    check("clear-cloud", rc == 0, out + err)
    for r in ("A", "B"):
        ah(r, "todo-sync", "disable")


TESTS = {
    "rpc": t_rpc,
    "basic": t_basic,
    "ops": t_ops,
    "concurrent": t_concurrent,
    "history": t_history_clear,
    "virtual": t_virtual,
    "push": t_push,
    "kill": t_kill,
    "idle": t_idle,
    "offline": t_offline,
    "worktree": t_worktree,
    "account": t_account,
    "rebuild": t_rebuild,
    "clear": t_clear,
    "cleanup": t_final_cleanup,
}

DEFAULT_ORDER = ["rpc", "basic", "ops", "concurrent", "history", "virtual", "worktree", "offline", "account", "rebuild", "clear", "push", "kill"]

if __name__ == "__main__":
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        sys.exit(0)
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
                log(f"  FAIL  {n} raised {type(e).__name__}: {e}")
    else:
        sys.exit(f"unknown command {args[0]!r}; see --help")
