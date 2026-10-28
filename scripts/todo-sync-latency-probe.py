#!/usr/bin/env python3
"""Measure how long a todo edit takes to reach the other Mac's todos.json (spec todo-sync section 16.1: "within
5 seconds when both Macs are online and active"), on the REAL iCloud zone with two real Macs.

  scripts/todo-sync-latency-probe.py run --peer <ssh target> [--peer-dir '~/Developer/AskHuman'] \
        [--local-dir <this Mac's checkout>] [--phases natural:2,active:3] [--timeout 240]
  scripts/todo-sync-latency-probe.py selftest      # the pure parsing / statistics code, no side effects

Every probe todo yields two samples: its creation on one Mac (landing in the other Mac's todos.json) and its removal
on the other Mac (landing back). Both Macs run a tiny watcher that notes when todos.json starts / stops containing
the probe tag; the clocks are aligned by ping-pong over the same channel; the helper logs of both Macs then split
each sample into: edit -> helper save done, save -> push arrival, push -> fetch done, fetch done -> file landed.

Phases: `natural` leaves both Macs as they are; `active` holds `caffeinate -u` on both (declares user activity and
keeps the display on) so the "both Macs active" case does not depend on someone moving a mouse.

Side effects on real data: probe todos ("AH-PROBE-<run>-NN ...") are added to the project of --local-dir /
--peer-dir and removed again. Removals are tombstones that sync never purges (a few hundred bytes each). The CLI
removes by index only, so the remove step looks the index up by tag right before `todo rm`, checks the text it
removed and re-adds any other todo it hit by mistake. Leave the project's todo list alone while it runs.
Needs passwordless ssh to the peer. Run it from the main checkout, not a Dev Instance worktree (ASKHUMAN_HOME is
stripped for the commands it starts). The result is also written to --out (default /tmp/ah-latency-probe.json).
"""
import argparse
import datetime
import json
import os
import random
import re
import shlex
import signal
import statistics
import subprocess
import sys
import threading
import time

LOG = os.path.expanduser("~/Library/Logs/AskHumanTodoSync/helper.log")
LINE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?)Z (\w+)(.*)$")
TAG_RE = re.compile(r"AH-PROBE-[0-9a-f]+-\d+")
PREFIX = "AH-PROBE-"
BIN = '"$HOME/.local/bin/AskHuman"'
CAFFEINATE_SECS = 1789  # distinctive, so the peer's copy can be found and killed by pkill -f
SSH_OPTS = [
    "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=5",
    "-o", "ControlMaster=auto", "-o", "ControlPath=/tmp/ah-probe-ssh-%C", "-o", "ControlPersist=60",
]

# Runs on both Macs (python3, stdlib only): prints a line whenever the set of probe tags in todos.json changes,
# answers `ping <id>` with the local clock for clock alignment, exits on `quit` or when stdin closes.
WATCHER = r'''
import json, os, re, sys, threading, time
PATH = os.path.expanduser("~/.askhuman/state/todos.json")
TAG = re.compile(r"AH-PROBE-[0-9a-f]+-\d+")
lock = threading.Lock()
def out(line):
    with lock:
        sys.stdout.write(line + "\n")
        sys.stdout.flush()
def reader():
    for line in sys.stdin:
        parts = line.split()
        if parts and parts[0] == "ping":
            out("pong %s %.6f" % (parts[1], time.time()))
        elif parts and parts[0] == "quit":
            break
    os._exit(0)
threading.Thread(target=reader, daemon=True).start()
last_sig = None
last_tags = None
out("ready %.6f" % time.time())
while True:
    try:
        st = os.stat(PATH)
    except OSError:
        time.sleep(0.05)
        continue
    sig = (st.st_mtime_ns, st.st_size, st.st_ino)
    if sig != last_sig:
        seen = time.time()
        try:
            with open(PATH) as f:
                data = json.load(f)
        except (OSError, ValueError):
            time.sleep(0.005)
            continue
        last_sig = sig
        found = set()
        for items in data.get("projects", {}).values():
            if isinstance(items, list):
                for t in items:
                    if isinstance(t, dict):
                        found.update(TAG.findall(t.get("text", "")))
        tags = sorted(found)
        if tags != last_tags:
            last_tags = tags
            out("S %.6f %.6f %s" % (seen, st.st_mtime_ns / 1e9, "|".join(tags) or "-"))
    time.sleep(0.01)
'''


# ---------------------------------------------------------------- pure helpers (covered by selftest)

def log_events(text):
    """Parse helper log lines into (epoch seconds, event name, {key: value})."""
    out = []
    for raw in text.splitlines():
        m = LINE.match(raw)
        if not m:
            continue
        ts = datetime.datetime.fromisoformat(m.group(1)).replace(tzinfo=datetime.timezone.utc).timestamp()
        fields = dict(kv.split("=", 1) for kv in m.group(3).split() if "=" in kv)
        out.append((ts, m.group(2), fields))
    return out


def parse_phases(spec):
    """`natural:2,active:3` -> [("natural", 2), ("active", 3)]."""
    phases = []
    for part in spec.split(","):
        name, _, rounds = part.partition(":")
        if name not in ("natural", "active") or not rounds.isdigit() or int(rounds) < 1:
            raise ValueError(f"bad phase {part!r} (use natural:N and/or active:N)")
        phases.append((name, int(rounds)))
    return phases


def parse_removed(output):
    """The text `todo rm` reports it removed ("... #3: <text>"), or None."""
    m = re.search(r"#(\d+): (.*)", output)
    return m.group(2).strip() if m else None


def percentile(values, p):
    values = sorted(values)
    if not values:
        return None
    k = (len(values) - 1) * p / 100.0
    lo, hi = int(k), min(int(k) + 1, len(values) - 1)
    return values[lo] + (values[hi] - values[lo]) * (k - lo)


def stats(values):
    """n / min / median / p90 / max and how many are within 5 s and 10 s."""
    values = [v for v in values if v is not None]
    if not values:
        return {"n": 0}
    return {
        "n": len(values),
        "min": min(values),
        "median": statistics.median(values),
        "p90": percentile(values, 90),
        "max": max(values),
        "le5": sum(1 for v in values if v <= 5.0),
        "le10": sum(1 for v in values if v <= 10.0),
    }


def first_at_or_after(events, name, since, pred=lambda f: True):
    """First (ts, fields) of event `name` with ts >= since."""
    for ts, n, f in events:
        if n == name and ts >= since and pred(f):
            return ts, f
    return None


def breakdown(src_log, dst_log, t_done, landed):
    """Split one sample into phases, using the helper logs already converted to one clock (seconds)."""
    out = {}
    save = first_at_or_after(src_log, "rpc_save", t_done - 0.05, lambda f: int(f.get("records", "0") or 0) >= 1)
    if not save:
        return out
    t_save = save[0]
    out["edit_to_saved"] = t_save - t_done
    out["save_ms"] = int(save[1].get("ms", "0") or 0)
    push = first_at_or_after(dst_log, "push", t_save - 1.0)
    fetch = first_at_or_after(dst_log, "rpc_fetch", t_save - 1.0, lambda f: int(f.get("changes", "0") or 0) >= 1)
    if push:
        out["saved_to_push"] = push[0] - t_save
    if fetch:
        out["fetch_ms"] = int(fetch[1].get("ms", "0") or 0)
        out["push_to_fetched"] = fetch[0] - (push[0] if push else t_save)
        out["fetched_to_landed"] = landed - fetch[0]
        tick = first_at_or_after(dst_log, "tick_done", fetch[0] - 0.01)
        out["via"] = tick[1].get("reason", "?") if tick else "?"
    return out


# ---------------------------------------------------------------- machines

class Watcher:
    """A todos.json watcher process (local or over ssh) with a reader thread."""

    def __init__(self, name, argv):
        self.name = name
        self.proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, text=True, bufsize=1)
        self.events = []  # (seen, mtime, frozenset(tags)), in the watcher's own clock
        self.tags = frozenset()
        self.pongs = {}
        self.lock = threading.Lock()
        self.ready = threading.Event()
        self.offset = 0.0  # remote clock minus local clock
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.proc.stdout:
            now = time.time()
            parts = line.rstrip("\n").split(" ", 3)
            if parts[0] == "ready":
                self.ready.set()
            elif parts[0] == "pong" and len(parts) >= 3:
                with self.lock:
                    self.pongs[parts[1]] = (float(parts[2]), now)
            elif parts[0] == "S" and len(parts) == 4:
                tags = frozenset(t for t in parts[3].split("|") if t and t != "-")
                with self.lock:
                    self.tags = tags
                    self.events.append((float(parts[1]), float(parts[2]), tags))

    def send(self, line):
        try:
            self.proc.stdin.write(line + "\n")
            self.proc.stdin.flush()
        except (BrokenPipeError, ValueError, OSError):
            pass

    def align(self, rounds=30):
        """remote - local clock offset from the smallest-RTT ping-pong; returns (rtt, offset)."""
        best = None
        for i in range(rounds):
            key = f"{time.time_ns()}-{i}"
            t0 = time.time()
            self.send(f"ping {key}")
            deadline = t0 + 5
            got = None
            while time.time() < deadline and got is None:
                with self.lock:
                    got = self.pongs.pop(key, None)
                if got is None:
                    time.sleep(0.001)
            if got is None:
                continue
            remote, t1 = got
            rtt = t1 - t0
            if best is None or rtt < best[0]:
                best = (rtt, remote - (t0 + t1) / 2.0)
            time.sleep(0.03)
        if best is None:
            raise RuntimeError(f"{self.name}: no ping answers")
        self.offset = best[1]
        return best

    def mark(self):
        with self.lock:
            return len(self.events)

    def wait(self, pred, start, timeout):
        """First event after index `start` whose tag set satisfies pred, else None (also if already true)."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            with self.lock:
                for ev in self.events[start:]:
                    if pred(ev[2]):
                        return ev
            time.sleep(0.005)
        return None

    def stop(self):
        self.send("quit")
        try:
            self.proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.proc.kill()


class Mac:
    """One Mac: where its todo CLI runs (cwd) and how to reach it."""

    def __init__(self, name, cwd, host=None):
        self.name = name
        self.cwd = cwd
        self.host = host
        self.watcher = None

    def start_watcher(self):
        if self.host is None:
            argv = [sys.executable, "-u", "-c", WATCHER]
        else:
            argv = ["ssh", *SSH_OPTS, self.host, "python3 -u -c " + shlex.quote(WATCHER)]
        self.watcher = Watcher(self.name, argv)
        if not self.watcher.ready.wait(20):
            raise RuntimeError(f"{self.name}: watcher did not start")

    def sh(self, script, timeout=90):
        """Run a shell snippet in the project dir. Returns (rc, stdout, end time already mapped to OUR clock)."""
        if self.host is None:
            env = {k: v for k, v in os.environ.items() if k != "ASKHUMAN_HOME"}
            p = subprocess.run(["/bin/sh", "-c", script], cwd=self.cwd, env=env, capture_output=True,
                               text=True, timeout=timeout)
            return p.returncode, p.stdout, time.time()
        remote = (f"cd {self.cwd} && {{ {script}\n}}; rc=$?; "
                  "perl -MTime::HiRes=time -e 'printf qq(\\nTS %.6f\\n), time'; exit $rc")
        p = subprocess.run(["ssh", *SSH_OPTS, self.host, remote], capture_output=True, text=True, timeout=timeout)
        m = re.search(r"\nTS ([0-9.]+)\n?$", p.stdout)
        if not m:
            return p.returncode, p.stdout, time.time()
        return p.returncode, p.stdout[: m.start()], float(m.group(1)) - self.watcher.offset

    def to_local(self, t):
        """A timestamp from this Mac's clock, on the orchestrator's clock."""
        return t - (self.watcher.offset if self.host else 0.0)

    def add_cmd(self, text):
        return f"{BIN} todo add -- {shlex.quote(text)}"

    def rm_cmd(self, tag):
        return (f"n=$({BIN} todo list | awk -v t={shlex.quote(tag)} "
                "'index($0, t) { s = $1; sub(/\\.$/, \"\", s); print s; exit }'); "
                f"[ -n \"$n\" ] || {{ echo NOTFOUND; exit 3; }}; {BIN} todo rm \"$n\"")

    def helper_log(self):
        if self.host is None:
            try:
                with open(LOG) as f:
                    return f.read()
            except OSError:
                return ""
        p = subprocess.run(["ssh", *SSH_OPTS, self.host, "cat ~/Library/Logs/AskHumanTodoSync/helper.log"],
                           capture_output=True, text=True, timeout=60)
        return p.stdout


# ---------------------------------------------------------------- run

def run(args):
    phases = parse_phases(args.phases)
    run_id = "%06x" % random.getrandbits(24)
    a = Mac("A", args.local_dir)
    b = Mac("B", args.peer_dir, args.peer)
    macs = (a, b)
    samples = []
    caffeinates = []
    notes = []
    state = {"seq": 0}

    def cleanup():
        # Remove any probe todo of this run that is still around (either Mac may hold it), then stop everything.
        for mac in macs:
            if mac.watcher is None:
                continue
            for tag in sorted(t for t in mac.watcher.tags if f"-{run_id}-" in t):
                rc, out, _ = mac.sh(mac.rm_cmd(tag))
                notes.append(f"cleanup: removed leftover {tag} on {mac.name} (rc={rc})")
        for proc in caffeinates:
            proc.terminate()
        if b.watcher is not None:
            subprocess.run(["ssh", *SSH_OPTS, b.host, f"pkill -f 'caffeinate -u -t {CAFFEINATE_SECS}' || true"],
                           capture_output=True, timeout=30)
        for mac in macs:
            if mac.watcher is not None:
                mac.watcher.stop()
        subprocess.run(["ssh", *SSH_OPTS, "-O", "exit", b.host], capture_output=True, timeout=10)

    def on_signal(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, on_signal)

    try:
        for mac in macs:
            mac.start_watcher()
        for mac in macs:
            rtt, off = mac.watcher.align()
            print(f"clock alignment {mac.name}: rtt {rtt * 1000:.1f} ms, offset {off * 1000:+.1f} ms", flush=True)
        t_start = time.time()

        def entity(phase, src, dst, counted=True):
            state["seq"] += 1
            tag = f"{PREFIX}{run_id}-{state['seq']:02d}"
            text = f"{tag} 同步探针，勿处理（自动删除）"
            # 1) create on src, wait until it lands in dst's todos.json
            mark = dst.watcher.mark()
            rc, out, t_done = src.sh(src.add_cmd(text))
            if rc != 0:
                raise RuntimeError(f"todo add failed on {src.name}: rc={rc}")
            ev = dst.watcher.wait(lambda tags: tag in tags, mark, args.timeout)
            record(samples, phase, "add", src, dst, tag, t_done, ev, counted)
            if ev is None:
                notes.append(f"{tag}: never landed on {dst.name} within {args.timeout}s; removing it on {src.name}")
                src.sh(src.rm_cmd(tag))
                return
            # 2) remove on dst right away, wait until it disappears from src's todos.json
            mark = src.watcher.mark()
            rc, out, t_done = dst.sh(dst.rm_cmd(tag))
            removed = parse_removed(out)
            if rc != 0 or removed is None:
                raise RuntimeError(f"todo rm failed on {dst.name}: rc={rc} out={out.strip()[:80]!r}")
            if tag not in removed:
                # Never leave the user's todo deleted: put the text back (loses its id / order, but not the text).
                notes.append(f"WRONG TODO REMOVED on {dst.name}; re-adding it (text length {len(removed)})")
                dst.sh(dst.add_cmd(removed))
                dst.sh(dst.rm_cmd(tag))
            ev = src.watcher.wait(lambda tags: tag not in tags, mark, args.timeout)
            record(samples, phase, "rm", dst, src, tag, t_done, ev, counted)

        print("warm-up (not counted) ...", flush=True)
        entity("warmup", a, b, counted=False)
        for phase, rounds in phases:
            if phase == "active":
                for mac in macs:
                    cmd = ["caffeinate", "-u", "-t", str(CAFFEINATE_SECS)]
                    if mac.host:
                        cmd = ["ssh", *SSH_OPTS, mac.host, " ".join(cmd)]
                    caffeinates.append(subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
                time.sleep(3)
            for r in range(rounds):
                for src, dst in ((a, b), (b, a)):
                    time.sleep(random.uniform(15, 25))
                    print(f"[{phase}] round {r + 1}/{rounds}: {src.name} -> {dst.name} ...", flush=True)
                    entity(phase, src, dst)
            if phase == "active":
                for proc in caffeinates:
                    proc.terminate()
                caffeinates.clear()
                subprocess.run(["ssh", *SSH_OPTS, b.host, f"pkill -f 'caffeinate -u -t {CAFFEINATE_SECS}' || true"],
                               capture_output=True, timeout=30)
        t_end = time.time()
        time.sleep(2)
        attach_breakdown(samples, a, b, t_start - 10, t_end + 10)
    finally:
        cleanup()

    report(samples, notes, args.out, run_id)


def record(samples, phase, kind, src, dst, tag, t_done, ev, counted):
    s = {"phase": phase, "kind": kind, "dir": f"{src.name}->{dst.name}", "tag": tag, "counted": counted,
         "src": src.name, "dst": dst.name, "t_done": t_done}
    if ev is None:
        s.update(landed=None, latency=None, latency_seen=None)
    else:
        seen, mtime = dst.to_local(ev[0]), dst.to_local(ev[1])
        s.update(landed=mtime, latency=mtime - t_done, latency_seen=seen - t_done)
    samples.append(s)


def attach_breakdown(samples, a, b, since, until):
    logs = {}
    for mac in (a, b):
        logs[mac.name] = [(mac.to_local(ts), n, f) for ts, n, f in log_events(mac.helper_log()) if since <= mac.to_local(ts) <= until]
    for s in samples:
        if s["landed"] is not None:
            s.update(breakdown(logs[s["src"]], logs[s["dst"]], s["t_done"], s["landed"]))


def fmt(v, width=6):
    return " " * (width - 1) + "-" if v is None else f"{v:{width}.1f}"


def report(samples, notes, out_path, run_id):
    lines = [f"todo-sync latency probe {run_id}: {len([s for s in samples if s['counted']])} counted samples",
             "phase    dir   kind  latency  edit>save save>push push>fetch fetch>file  via"]
    for s in samples:
        if not s["counted"] and s["phase"] != "warmup":
            continue
        lines.append(f"{s['phase']:8} {s['dir']:5} {s['kind']:4} {fmt(s['latency'], 8)}  {fmt(s.get('edit_to_saved'), 8)}"
                     f" {fmt(s.get('saved_to_push'), 8)} {fmt(s.get('push_to_fetched'), 9)} {fmt(s.get('fetched_to_landed'), 9)}"
                     f"  {s.get('via', '-')}{'  (warm-up)' if s['phase'] == 'warmup' else ''}")
    lines.append("")
    for phase in ("natural", "active"):
        for direction in ("A->B", "B->A", None):
            vals = [s["latency"] for s in samples if s["counted"] and s["phase"] == phase
                    and (direction is None or s["dir"] == direction)]
            if not vals:
                continue
            st = stats(vals)
            if st["n"] == 0:
                lines.append(f"{phase:8} {direction or 'both':5}: no sample landed")
                continue
            lines.append(f"{phase:8} {direction or 'both':5}: n={st['n']} min {st['min']:.1f}s median {st['median']:.1f}s "
                         f"p90 {st['p90']:.1f}s max {st['max']:.1f}s  <=5s: {st['le5']}/{st['n']}  <=10s: {st['le10']}/{st['n']}")
        missed = [s for s in samples if s["counted"] and s["phase"] == phase and s["latency"] is None]
        if missed:
            lines.append(f"{phase:8} samples that never landed in time: {len(missed)}")
    lines.extend(notes)
    text = "\n".join(lines)
    print(text)
    with open(out_path, "w") as f:
        json.dump({"run": run_id, "samples": samples, "notes": notes}, f, indent=1)
    with open(out_path.replace(".json", ".txt"), "w") as f:
        f.write(text + "\n")


# ---------------------------------------------------------------- selftest

def selftest():
    sample = ("2026-10-28T16:06:32.557Z rpc_save conflicts=0 ms=619 records=1\n"
              "2026-10-28T16:06:42.645Z push ours=1\n"
              "2026-10-28T16:06:44.882Z rpc_fetch changes=1 deletions=0 fromScratch=0 moreComing=0 ms=1304\n"
              "2026-10-28T16:06:44.978Z tick_done exit=0 ms=2028 reason=push\n"
              "garbage line\n")
    ev = log_events(sample)
    assert [e[1] for e in ev] == ["rpc_save", "push", "rpc_fetch", "tick_done"], ev
    t0 = ev[0][0]
    bd = breakdown(ev, ev, t0 - 1.5, t0 + 12.5)
    assert abs(bd["edit_to_saved"] - 1.5) < 1e-6, bd
    assert abs(bd["saved_to_push"] - 10.088) < 1e-3, bd
    assert abs(bd["push_to_fetched"] - 2.237) < 1e-3 and bd["fetch_ms"] == 1304 and bd["via"] == "push", bd
    assert breakdown([], [], 0, 1) == {}
    assert parse_phases("natural:2,active:3") == [("natural", 2), ("active", 3)]
    for bad in ("", "natural", "active:0", "other:1", "natural:x"):
        try:
            parse_phases(bad)
            raise AssertionError(bad)
        except ValueError:
            pass
    assert parse_removed("已删除待办 #3: AH-PROBE-ab12cd-01 同步探针") == "AH-PROBE-ab12cd-01 同步探针"
    assert parse_removed("Removed todo #12: something else") == "something else"
    assert parse_removed("NOTFOUND") is None
    st = stats([1.0, 2.0, 3.0, 4.0, 11.0, None])
    assert st["n"] == 5 and st["median"] == 3.0 and st["le5"] == 4 and st["le10"] == 4 and st["max"] == 11.0, st
    assert stats([None]) == {"n": 0}
    assert abs(percentile([1, 2, 3, 4, 5], 90) - 4.6) < 1e-9
    assert TAG_RE.findall("AH-PROBE-ab12cd-01 and AH-PROBE-ab12cd-02, no AH-PROBE-xyz") == ["AH-PROBE-ab12cd-01", "AH-PROBE-ab12cd-02"]
    w = Watcher.__new__(Watcher)  # wait() logic without a process
    w.lock = threading.Lock()
    w.events = [(1.0, 0.9, frozenset({"t1"})), (2.0, 1.9, frozenset())]
    assert w.wait(lambda tags: "t1" in tags, 0, 0.05)[0] == 1.0
    assert w.wait(lambda tags: "t1" not in tags, 1, 0.05)[0] == 2.0
    assert w.wait(lambda tags: "t1" in tags, 2, 0.05) is None
    compile(WATCHER, "watcher", "exec")
    print("selftest ok")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("selftest")
    p = sub.add_parser("run")
    p.add_argument("--peer", required=True, help="ssh target of the other Mac")
    p.add_argument("--peer-dir", default="~/Developer/AskHuman", help="project directory on the peer (unquoted, ~ allowed)")
    p.add_argument("--local-dir", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    p.add_argument("--phases", default="natural:2,active:3")
    p.add_argument("--timeout", type=float, default=240.0, help="seconds to wait for each landing")
    p.add_argument("--out", default="/tmp/ah-latency-probe.json")
    args = parser.parse_args()
    if args.cmd == "selftest":
        selftest()
    else:
        run(args)


if __name__ == "__main__":
    main()
