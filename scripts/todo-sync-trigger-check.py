#!/usr/bin/env python3
"""Check the todo-sync helper's wake / unlock / network triggers on THIS Mac (spec todo-sync section 6,
plan P6 "E8"). Needs a real session: the Mac goes to sleep or the Wi-Fi is switched off while it runs.

  scripts/todo-sync-trigger-check.py status     # baseline only, touches nothing
  scripts/todo-sync-trigger-check.py sleep      # pmset sleepnow; wake the Mac by hand (key / lid)
  scripts/todo-sync-trigger-check.py network    # Wi-Fi off for 15 s, then on again
  scripts/todo-sync-trigger-check.py selftest   # the pure log-parsing / verdict code, no side effects

Prerequisites: sync is enabled on this Mac (`AskHuman todo-sync status` says "on") and the helper runs.
What counts as a pass: after the Mac woke (or the network came back) the helper log has a `tick_done exit=0`
whose reason is `wake` / `unlock` (sleep) or `network` (network), and `lastPullOkMs` moved past that moment.

Run it detached (`nohup scripts/todo-sync-trigger-check.py sleep &`): the shell that started it may lose its
connection while the Mac sleeps / the network is down. The report is also written to /tmp/ah-trigger-check.out.
Run it from outside a Dev Instance worktree (the script strips ASKHUMAN_HOME and runs AskHuman from $HOME).
"""
import atexit
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.request

LOG = os.path.expanduser("~/Library/Logs/AskHumanTodoSync/helper.log")
REPORT = "/tmp/ah-trigger-check.out"
LINE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?)Z (\w+)(.*)$")


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


def finished_ticks(events, since):
    """Ticks that finished at or after `since`: [(finished_at, started_at, reason, exit)]."""
    ticks = []
    for ts, name, f in events:
        if name == "tick_done" and ts >= since:
            ms = int(f.get("ms", "0") or 0)
            ticks.append((ts, ts - ms / 1000.0, f.get("reason", "?"), int(f.get("exit", "-1"))))
    return ticks


def verdict(ticks, wanted_reasons, last_pull_ok_s, since):
    """(ok, text): was there a good tick for one of the wanted reasons, and did the pull clock advance."""
    good = [t for t in ticks if t[2] in wanted_reasons and t[3] == 0]
    if not good:
        seen = ", ".join(f"{t[2]}/exit={t[3]}" for t in ticks) or "none"
        return False, f"no tick with reason {'/'.join(sorted(wanted_reasons))} and exit=0 (ticks seen: {seen})"
    first = good[0]
    lag = first[1] - since
    if last_pull_ok_s < since:
        return False, f"a {first[2]} tick ran but lastPullOkMs did not advance (the pull failed or timed out)"
    return True, f"{first[2]} tick started {lag:.1f} s after the event and took {first[0] - first[1]:.1f} s"


def selftest():
    text = (
        "2026-10-28T15:34:17.666Z tick_done exit=0 ms=4195 reason=wake\n"
        "2026-10-28T15:34:19.309Z tick_done exit=0 ms=499 reason=file\n"
        "2026-10-28T15:39:08.675Z push ours=1\n"
        "garbage line\n"
        "2026-10-28T15:40:00.000Z tick_done exit=-1 ms=50 reason=network\n"
    )
    ev = log_events(text)
    assert len(ev) == 4, ev
    base = datetime.datetime(2026, 10, 28, 15, 34, 10, tzinfo=datetime.timezone.utc).timestamp()
    ticks = finished_ticks(ev, base)
    assert [t[2] for t in ticks] == ["wake", "file", "network"], ticks
    ok, why = verdict(ticks, {"wake", "unlock"}, base + 8, base)
    assert ok and "wake tick started" in why, why
    ok, why = verdict(ticks, {"wake", "unlock"}, base - 5, base)
    assert not ok and "did not advance" in why, why
    ok, why = verdict(ticks, {"network"}, base + 60, base)
    assert not ok and "no tick with reason network" in why, why  # exit=-1 means "not enabled", never a pass
    assert finished_ticks(ev, base + 1000) == []
    print("selftest ok")


def ask_human(*args):
    env = {k: v for k, v in os.environ.items() if k != "ASKHUMAN_HOME"}
    exe = shutil.which("AskHuman") or os.path.expanduser("~/.local/bin/AskHuman")
    return subprocess.run([exe, *args], capture_output=True, text=True, env=env, cwd=os.path.expanduser("~"), timeout=60)


def status():
    r = ask_human("todo-sync", "status", "--json")
    try:
        return json.loads(r.stdout)
    except ValueError:
        sys.exit(f"cannot read `AskHuman todo-sync status --json`: {r.stdout[:200]!r} {r.stderr[:200]!r}")


def show_status(label, s):
    last = s.get("lastTick") or {}
    emit(f"{label}: enabled={s.get('enabled')} helper={(s.get('helper') or {}).get('state')} paused={s.get('paused')} "
         f"unsynced={s.get('unsynced')} lastPullOkMs={s.get('lastPullOkMs')} "
         f"lastTick={last.get('reason')}/{last.get('outcome')}")


def emit(msg):
    line = f"{time.strftime('%H:%M:%S')} {msg}"
    print(line, flush=True)
    with open(REPORT, "a") as f:
        f.write(line + "\n")


def read_log():
    try:
        with open(LOG, errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def precheck():
    s = status()
    show_status("baseline", s)
    if not s.get("supported") or not s.get("enabled"):
        sys.exit("sync is not enabled here (run from outside a Dev Instance worktree, or `AskHuman todo-sync enable` first)")
    if (s.get("helper") or {}).get("state") != "running":
        sys.exit("the helper is not running")
    return s


def wait_for_ticks(since, reasons, seconds):
    """Poll until a good tick for `reasons` finished after `since` (epoch seconds), or the time is up."""
    deadline = time.time() + seconds
    while time.time() < deadline:
        ticks = finished_ticks(log_events(read_log()), since)
        if any(t[2] in reasons and t[3] == 0 for t in ticks):
            time.sleep(2)  # let the status files settle
            break
        time.sleep(2)
    return finished_ticks(log_events(read_log()), since)


def report(title, ticks, reasons, since):
    s = status()
    show_status("after", s)
    ok, why = verdict(ticks, reasons, (s.get("lastPullOkMs") or 0) / 1000.0, since)
    emit(f"{title}: {'PASS' if ok else 'FAIL'}: {why}")
    for t in ticks:
        emit(f"  tick {t[2]} exit={t[3]} started +{t[1] - since:.1f} s, took {t[0] - t[1]:.1f} s")
    return ok


def check_sleep():
    precheck()
    open(REPORT, "w").close()
    emit("going to sleep in 3 s; wake the Mac by hand (key press / lid)")
    time.sleep(3)
    start = time.time()
    subprocess.run(["pmset", "sleepnow"], check=False)
    prev = time.time()
    while True:  # a wall-clock jump means the system slept in between
        time.sleep(1)
        now = time.time()
        if now - prev > 8:
            break
        prev = now
        if now - start > 600:
            emit("the Mac did not sleep within 10 minutes; giving up")
            return False
    woke = now
    emit(f"woke: the loop was away for {now - prev:.0f} s")
    ticks = wait_for_ticks(woke - 2, {"wake", "unlock"}, 90)
    return report("sleep/wake", ticks, {"wake", "unlock"}, woke)


def wifi_device():
    out = subprocess.run(["networksetup", "-listallhardwareports"], capture_output=True, text=True).stdout
    m = re.search(r"Hardware Port: (?:Wi-Fi|AirPort)\s*\nDevice: (\w+)", out)
    return m.group(1) if m else None


def online():
    try:
        urllib.request.urlopen("https://www.apple.com", timeout=3).read(1)
        return True
    except Exception:
        return False


def check_network():
    precheck()
    dev = wifi_device()
    if not dev:
        sys.exit("no Wi-Fi hardware port found")
    if not online():
        sys.exit("this Mac is not online right now; the test needs a working network to begin with")
    open(REPORT, "w").close()
    atexit.register(lambda: subprocess.run(["networksetup", "-setairportpower", dev, "on"], check=False))
    emit(f"Wi-Fi ({dev}) off for 15 s")
    subprocess.run(["networksetup", "-setairportpower", dev, "off"], check=True)
    time.sleep(15)
    subprocess.run(["networksetup", "-setairportpower", dev, "on"], check=True)
    switched_on = time.time()
    back = None
    for _ in range(120):
        if online():
            back = time.time()
            break
        time.sleep(1)
    if back is None:
        emit("the network did not come back within 120 s")
        return False
    emit(f"network usable again {back - switched_on:.0f} s after switching Wi-Fi on")
    ticks = wait_for_ticks(back - 2, {"network"}, 90)
    return report("network restore", ticks, {"network"}, back)


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "selftest":
        selftest()
    elif cmd == "status":
        precheck()
    elif cmd == "sleep":
        sys.exit(0 if check_sleep() else 1)
    elif cmd == "network":
        sys.exit(0 if check_network() else 1)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
