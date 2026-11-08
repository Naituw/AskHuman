#!/usr/bin/env python3
"""Drive the real Duo fold transitions requested by AskHumanFoldNavigationUITests.

First build-for-testing, then pass its .xctestrun and the Duo UDID. The temporary
run file stays beside the original so __TESTROOT__ retains its meaning.
"""
import argparse
import json
from pathlib import Path
import plistlib
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("xctestrun", type=Path)
    parser.add_argument("device")
    parser.add_argument("--result", required=True)
    parser.add_argument("--only", action="append", help="Run only this method of AskHumanFoldNavigationUITests (repeatable).")
    args = parser.parse_args()

    data = plistlib.loads(args.xctestrun.read_bytes())
    runner = data["AskHumanUITests"]["TestHostBundleIdentifier"]
    # These verify navigation, not performance. Avoid Xcode's separate signpost collector.
    data["AskHumanUITests"]["UITargetAppPerformanceAntipatternCheckerEnabled"] = False

    def fold(command, nonce=None):
        result = subprocess.run(["duoctl", "-d", args.device, command, "--duration", "0.5"],
                                text=True, capture_output=True, timeout=60)
        expected = "inner" if command == "open" else "cover"
        deadline = time.monotonic() + 20
        # The animation can finish after duoctl's short display-switch deadline.
        # Trust the observed screen, never a timeout alone or an assumed success.
        while True:
            observed = subprocess.run(["duoctl", "-d", args.device, "state"],
                                      text=True, capture_output=True, timeout=20)
            if observed.returncode == 0:
                state = json.loads(observed.stdout)
                if state["activeScreen"] == expected:
                    break
            if time.monotonic() >= deadline:
                raise RuntimeError(f"Duo did not reach {expected}: {result.stderr or result.stdout}; {observed.stderr or observed.stdout}")
            time.sleep(0.5)
        if nonce:
            container = subprocess.check_output(["xcrun", "simctl", "get_app_container", args.device, runner, "data"],
                                                text=True, timeout=20).strip()
            acknowledgement = Path(container) / "Documents" / "duo-fold-state.json"
            acknowledgement.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.NamedTemporaryFile(mode="w", dir=acknowledgement.parent, delete=False) as completed:
                json.dump({**state, "nonce": nonce}, completed)
            Path(completed.name).replace(acknowledgement)
        print(f"Duo: {state['activeScreen']} {state['sizePoints']}", flush=True)

    data["AskHumanUITests"].setdefault("EnvironmentVariables", {})["AH_DUO_FOLD_TESTS"] = "YES"
    with tempfile.NamedTemporaryFile(prefix="duo-", suffix=".xctestrun", dir=args.xctestrun.parent) as temporary:
        temporary.write(plistlib.dumps(data))
        temporary.flush()
        fold("open")
        selections = ["-only-testing:AskHumanUITests/AskHumanFoldNavigationUITests" + ("/" + name if name else "")
                      for name in (args.only or [""])]
        process = subprocess.Popen([
            "xcodebuild", "test-without-building", "-xctestrun", temporary.name,
            "-destination", f"platform=iOS Simulator,id={args.device}",
            "-resultBundlePath", args.result, "-parallel-testing-enabled", "NO",
            "-collect-test-diagnostics", "never",
        ] + selections, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
        failures = 0
        try:
            for line in process.stdout:
                print(line, end="", flush=True)
                if "Added attachment named 'missing-" in line:
                    # XCUIScreen.main can capture the inactive cover display while the inner display
                    # is active. Keep the actual panel beside the result before XCTest closes the app.
                    failures += 1
                    result = Path(args.result)
                    screenshot = result.with_name(f"{result.stem}-missing-{failures}.png")
                    try:
                        captured = subprocess.run(["duoctl", "-d", args.device, "screenshot", str(screenshot)],
                                                  text=True, capture_output=True, timeout=20)
                        print(f"Active display capture: {screenshot if captured.returncode == 0 else captured.stderr}", flush=True)
                    except subprocess.TimeoutExpired:
                        print("Active display capture timed out; keeping the XCTest failure.", flush=True)
                marker = "AH_DUO_FOLD_COMMAND "
                if marker in line:
                    command, nonce = line.split(marker, 1)[1].split()
                    if command in ("open", "close"):
                        fold(command, nonce)
            return process.wait()
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=30)


if __name__ == "__main__":
    raise SystemExit(main())
