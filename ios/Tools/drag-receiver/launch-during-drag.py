#!/usr/bin/env python3
"""Activate the receiver while XCTest holds a real drag finger down."""
import argparse
import json
from pathlib import Path
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--device", required=True)
parser.add_argument("--count", type=int, default=4)
parser.add_argument("--delay", type=float, default=1)
parser.add_argument("--timeout", type=float, default=240)
args = parser.parse_args()
devices = json.loads(subprocess.check_output(["xcrun", "simctl", "list", "devices", "--json"]))
device = next(d for group in devices["devices"].values() for d in group if d["udid"] == args.device)
root = Path(device["dataPath"]) / "Containers" / "Data" / "Application"
# The mock app writes only when its drag provider is created by a real lift.
for signal in root.glob("*/tmp/askhuman-drag-ready"):
    signal.unlink()
print("Waiting for the mock app's real drag lifts", flush=True)
deadline = time.monotonic() + args.timeout
received = 0
while received < args.count and time.monotonic() < deadline:
    for signal in root.glob("*/tmp/askhuman-drag-ready"):
        print("Lifted type: " + signal.read_text(), flush=True)
        signal.unlink()
        time.sleep(args.delay)
        subprocess.run(["xcrun", "simctl", "launch", args.device, "com.naituw.askhuman.drag-receiver"], check=True)
        received += 1
        print(f"Activated receiver for drag {received}/{args.count}", flush=True)
    time.sleep(0.1)
if received != args.count:
    raise SystemExit(f"Expected {args.count} drags, saw {received}")
